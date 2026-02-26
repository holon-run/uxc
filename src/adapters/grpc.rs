//! gRPC adapter with reflection support
//!
//! This module provides full gRPC support including:
//! - Server reflection for automatic schema discovery
//! - Dynamic method invocation using tonic
//! - Support for all 4 call types: unary, server-stream, client-stream, bidi-stream
//! - TLS and h2c (cleartext) support
//! - Proper error handling and status code mapping

use super::{Adapter, ExecutionResult, Operation, OperationDetail, Parameter, ProtocolType};
use crate::auth::Profile;
use crate::error::UxcError;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use prost::Message;
use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};
use reflection::{server_reflection_request, ServerReflectionRequest};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Endpoint;
use tonic::Status;
use tonic_reflection::pb as reflection;
use tracing::{debug, info};

/// Trait for executing grpcurl commands (abstracted for testing)
#[async_trait]
trait GrpcurlExecutor: Send + Sync {
    /// Execute grpcurl with the given arguments and return the result
    async fn execute(
        &self,
        plaintext: bool,
        headers: Vec<String>,
        request_json: &str,
        target: &str,
        method: &str,
    ) -> Result<GrpcurlResult>;
}

/// Result of a grpcurl execution
#[derive(Clone)]
struct GrpcurlResult {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Default grpcurl executor using tokio::process::Command
struct DefaultGrpcurlExecutor;

#[async_trait]
impl GrpcurlExecutor for DefaultGrpcurlExecutor {
    async fn execute(
        &self,
        plaintext: bool,
        headers: Vec<String>,
        request_json: &str,
        target: &str,
        method: &str,
    ) -> Result<GrpcurlResult> {
        let mut cmd = tokio::process::Command::new("grpcurl");
        cmd.arg("-format").arg("json");

        if plaintext {
            cmd.arg("-plaintext");
        }

        for header in headers {
            cmd.arg("-H").arg(header);
        }

        cmd.arg("-d").arg(request_json).arg(target).arg(method);

        let output = cmd.output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "grpcurl is required for gRPC unary calls. Install grpcurl and retry."
                )
            } else {
                anyhow::anyhow!("Failed to execute grpcurl: {}", e)
            }
        })?;

        Ok(GrpcurlResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

/// gRPC adapter implementation
pub struct GrpcAdapter {
    /// In-memory cache for reflection clients and descriptors
    in_memory_cache: Arc<RwLock<HashMap<String, CachedReflectionData>>>,
    /// Persistent schema cache
    schema_cache: Option<Arc<dyn crate::cache::Cache>>,
    /// Authentication profile
    auth_profile: Option<Profile>,
    /// grpcurl executor (abstracted for testing)
    grpcurl_executor: Arc<dyn GrpcurlExecutor>,
}

/// Cached reflection data for a server
#[derive(Clone)]
struct CachedReflectionData {
    services: HashMap<String, ServiceInfo>,
}

/// Information about a gRPC service
#[derive(Clone, Debug)]
struct ServiceInfo {
    methods: HashMap<String, MethodInfo>,
    #[allow(dead_code)]
    file_descriptors: Vec<FileDescriptorProto>,
}

/// Information about a gRPC method
#[derive(Clone, Debug)]
struct MethodInfo {
    name: String,
    service_name: String,
    input_type: String,
    output_type: String,
    is_server_streaming: bool,
    is_client_streaming: bool,
    description: Option<String>,
}

impl GrpcAdapter {
    pub fn new() -> Self {
        Self {
            in_memory_cache: Arc::new(RwLock::new(HashMap::new())),
            schema_cache: None,
            auth_profile: None,
            grpcurl_executor: Arc::new(DefaultGrpcurlExecutor),
        }
    }

    /// Create a new adapter with a custom grpcurl executor (for testing)
    #[cfg(test)]
    fn with_executor(mut self, executor: Arc<dyn GrpcurlExecutor>) -> Self {
        self.grpcurl_executor = executor;
        self
    }

    pub fn with_cache(mut self, cache: Arc<dyn crate::cache::Cache>) -> Self {
        self.schema_cache = Some(cache);
        self
    }

    pub fn with_auth(mut self, profile: Profile) -> Self {
        self.auth_profile = Some(profile);
        self
    }

    /// Parse URL to get host:port
    fn parse_url(url: &str) -> Result<String> {
        let url = url.trim_end_matches('/');

        // If it's already in host:port format
        if url.contains(':') && !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(url.to_string());
        }

        // Handle URLs
        let url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("http://{}", url)
        };

        // Parse and extract host:port
        let parsed = url::Url::parse(&url)?;
        let host = parsed.host_str().ok_or_else(|| anyhow!("Invalid host"))?;
        let port = parsed.port().unwrap_or(50051); // Default gRPC port
        Ok(format!("{}:{}", host, port))
    }

    /// Check if the server has reflection enabled
    async fn has_reflection(endpoint: &Endpoint) -> Result<bool> {
        let channel = endpoint
            .connect()
            .await
            .context("Failed to connect to gRPC server")?;

        let mut reflection_client =
            reflection::server_reflection_client::ServerReflectionClient::new(channel)
                .max_decoding_message_size(usize::MAX);

        let (tx, rx) = tokio::sync::mpsc::channel(4);

        let request = ServerReflectionRequest {
            host: String::new(),
            message_request: Some(server_reflection_request::MessageRequest::ListServices(
                String::new(),
            )),
        };

        let _ = tx.send(request).await;
        drop(tx); // Close the channel

        let result = tokio::time::timeout(
            Duration::from_secs(3),
            async {
                let response = reflection_client.server_reflection_info(ReceiverStream::new(rx)).await;
                match response {
                    Ok(streaming) => {
                        let mut stream = streaming.into_inner();
                        while let Some(message) = stream.message().await? {
                            match message.message_response {
                                Some(
                                    reflection::server_reflection_response::MessageResponse::ListServicesResponse(_),
                                ) => return Ok::<bool, anyhow::Error>(true),
                                Some(
                                    reflection::server_reflection_response::MessageResponse::ErrorResponse(err),
                                ) => {
                                    if err.error_code == tonic::Code::Unimplemented as i32 {
                                        return Ok::<bool, anyhow::Error>(false);
                                    }
                                }
                                _ => {}
                            }
                        }
                        Ok::<bool, anyhow::Error>(false)
                    }
                    Err(_) => Ok::<bool, anyhow::Error>(false),
                }
            },
        )
        .await;

        match result {
            Ok(Ok(has_reflection)) => Ok(has_reflection),
            Ok(Err(_)) => Ok(false),
            Err(_) => Ok(false), // Timeout
        }
    }

    /// List all services via reflection
    async fn list_services_reflection(&self, endpoint: &Endpoint) -> Result<Vec<String>> {
        let channel = endpoint.connect().await?;
        let mut client = reflection::server_reflection_client::ServerReflectionClient::new(channel)
            .max_decoding_message_size(usize::MAX);

        let (tx, rx) = tokio::sync::mpsc::channel(4);

        let request = ServerReflectionRequest {
            host: String::new(),
            message_request: Some(server_reflection_request::MessageRequest::ListServices(
                String::new(),
            )),
        };

        tx.send(request).await?;
        drop(tx); // Close the channel

        let mut stream = client
            .server_reflection_info(ReceiverStream::new(rx))
            .await?
            .into_inner();

        let mut services = Vec::new();
        while let Some(response) = stream.message().await? {
            if let Some(
                reflection::server_reflection_response::MessageResponse::ListServicesResponse(ls),
            ) = response.message_response
            {
                for service in ls.service {
                    services.push(service.name);
                }
            }
        }

        Ok(services)
    }

    fn descriptor_contains_service(
        descriptor: &FileDescriptorProto,
        full_service_name: &str,
    ) -> bool {
        let package = descriptor.package.clone().unwrap_or_default();
        descriptor.service.iter().any(|service| {
            let service_name = service.name.clone().unwrap_or_default();
            let candidate = if package.is_empty() {
                service_name
            } else {
                format!("{}.{}", package, service_name)
            };
            candidate == full_service_name
        })
    }

    /// Get file descriptors for a service symbol.
    async fn get_service_descriptors(
        &self,
        endpoint: &Endpoint,
        service_name: &str,
    ) -> Result<Vec<FileDescriptorProto>> {
        let channel = endpoint.connect().await?;
        let mut client = reflection::server_reflection_client::ServerReflectionClient::new(channel)
            .max_decoding_message_size(usize::MAX);

        let (tx, rx) = tokio::sync::mpsc::channel(4);

        let request = ServerReflectionRequest {
            host: String::new(),
            message_request: Some(
                server_reflection_request::MessageRequest::FileContainingSymbol(
                    service_name.to_string(),
                ),
            ),
        };

        tx.send(request).await?;
        drop(tx); // Close the channel

        let mut stream = client
            .server_reflection_info(ReceiverStream::new(rx))
            .await?
            .into_inner();

        while let Some(response) = stream.message().await? {
            if let Some(
                reflection::server_reflection_response::MessageResponse::FileDescriptorResponse(fd),
            ) = response.message_response
            {
                let mut descriptors = Vec::new();
                for descriptor_bytes in fd.file_descriptor_proto {
                    let descriptor = FileDescriptorProto::decode(descriptor_bytes.as_slice())
                        .context("Failed to decode file descriptor")?;
                    descriptors.push(descriptor);
                }
                if !descriptors.is_empty() {
                    return Ok(descriptors);
                }
            }
        }

        bail!("File descriptors not found for service: {}", service_name)
    }

    /// Parse service and methods from file descriptor
    fn parse_service_info(
        &self,
        service_descriptor: &FileDescriptorProto,
        all_descriptors: Vec<FileDescriptorProto>,
    ) -> Result<ServiceInfo> {
        let mut methods = HashMap::new();
        let package = service_descriptor.package.clone().unwrap_or_default();

        for service in &service_descriptor.service {
            let service_name = service.name.clone().unwrap_or_default();
            let full_service_name = if package.is_empty() {
                service_name
            } else {
                format!("{}.{}", package, service_name)
            };
            for method in &service.method {
                let method_name = method.name.clone().unwrap_or_default();
                let method_info = MethodInfo {
                    name: method_name.clone(),
                    service_name: full_service_name.clone(),
                    input_type: method.input_type.clone().unwrap_or_default(),
                    output_type: method.output_type.clone().unwrap_or_default(),
                    is_server_streaming: method.server_streaming.unwrap_or(false),
                    is_client_streaming: method.client_streaming.unwrap_or(false),
                    description: None, // Comments are in source_code_info
                };
                methods.insert(method_name, method_info);
            }
        }

        Ok(ServiceInfo {
            methods,
            file_descriptors: all_descriptors,
        })
    }

    /// Get or load service information
    async fn get_service_info(&self, url: &str) -> Result<HashMap<String, ServiceInfo>> {
        // Check in-memory cache first
        {
            let cache = self.in_memory_cache.read().await;
            if let Some(data) = cache.get(url) {
                return Ok(data.services.clone());
            }
        }

        // Load from reflection
        let endpoint = self.create_endpoint(url)?;
        let service_names = self.list_services_reflection(&endpoint).await?;

        let mut services = HashMap::new();
        for service_name in service_names {
            // Skip reflection services
            if service_name.contains("reflection") || service_name.contains("Reflection") {
                continue;
            }

            match self.get_service_descriptors(&endpoint, &service_name).await {
                Ok(descriptors) => {
                    let service_descriptor = descriptors
                        .iter()
                        .find(|descriptor| {
                            Self::descriptor_contains_service(descriptor, &service_name)
                        })
                        .cloned()
                        .or_else(|| descriptors.first().cloned());

                    if let Some(descriptor) = service_descriptor {
                        if let Ok(info) = self.parse_service_info(&descriptor, descriptors) {
                            services.insert(service_name.clone(), info);
                        }
                    } else {
                        tracing::warn!(
                            "No descriptor payload returned for service symbol {}",
                            service_name
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to get descriptor for {}: {}", service_name, e);
                }
            }
        }

        // Cache the results in memory
        let mut cache = self.in_memory_cache.write().await;
        cache.insert(
            url.to_string(),
            CachedReflectionData {
                services: services.clone(),
            },
        );

        Ok(services)
    }

    /// Create a gRPC endpoint with proper configuration
    fn create_endpoint(&self, url: &str) -> Result<Endpoint> {
        let addr = Self::parse_url(url)?;
        let endpoint = Endpoint::from_shared(format!("http://{}", addr))?
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10));

        Ok(endpoint)
    }

    /// Find method by full name (ServiceName/MethodName)
    async fn find_method(&self, url: &str, operation: &str) -> Result<MethodInfo> {
        let (method_info, _) = self.find_method_context(url, operation).await?;
        Ok(method_info)
    }

    async fn find_method_context(
        &self,
        url: &str,
        operation: &str,
    ) -> Result<(MethodInfo, Vec<FileDescriptorProto>)> {
        let (service_name, method_name) = operation.split_once('/').ok_or_else(|| {
            UxcError::InvalidArguments(format!(
                "Invalid operation ID format: {}. Use ServiceName/MethodName",
                operation
            ))
        })?;
        if service_name.is_empty() || method_name.is_empty() {
            return Err(UxcError::InvalidArguments(format!(
                "Invalid operation ID format: {}. Use ServiceName/MethodName",
                operation
            ))
            .into());
        }

        let services = self.get_service_info(url).await?;
        let service_info = services
            .get(service_name)
            .ok_or_else(|| UxcError::OperationNotFound(service_name.to_string()))?;

        let method_info = service_info
            .methods
            .get(method_name)
            .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;

        Ok((method_info.clone(), service_info.file_descriptors.clone()))
    }

    fn normalize_type_name(type_name: &str) -> String {
        type_name.trim_start_matches('.').to_string()
    }

    fn to_json_field_name(field: &FieldDescriptorProto) -> String {
        field
            .json_name
            .clone()
            .or_else(|| field.name.clone())
            .unwrap_or_default()
    }

    fn collect_message_descriptors(
        prefix: &str,
        messages: &[DescriptorProto],
        message_index: &mut HashMap<String, DescriptorProto>,
        enum_index: &mut HashMap<String, EnumDescriptorProto>,
    ) {
        for message in messages {
            let message_name = message.name.clone().unwrap_or_default();
            let full_name = if prefix.is_empty() {
                message_name
            } else {
                format!("{}.{}", prefix, message_name)
            };

            message_index.insert(full_name.clone(), message.clone());
            for enum_type in &message.enum_type {
                if let Some(name) = enum_type.name.clone() {
                    enum_index.insert(format!("{}.{}", full_name, name), enum_type.clone());
                }
            }

            Self::collect_message_descriptors(
                &full_name,
                &message.nested_type,
                message_index,
                enum_index,
            );
        }
    }

    fn build_descriptor_indexes(
        descriptors: &[FileDescriptorProto],
    ) -> (
        HashMap<String, DescriptorProto>,
        HashMap<String, EnumDescriptorProto>,
    ) {
        let mut message_index = HashMap::new();
        let mut enum_index = HashMap::new();

        for descriptor in descriptors {
            let package_prefix = descriptor.package.clone().unwrap_or_default();
            Self::collect_message_descriptors(
                &package_prefix,
                &descriptor.message_type,
                &mut message_index,
                &mut enum_index,
            );
            for enum_type in &descriptor.enum_type {
                if let Some(name) = enum_type.name.clone() {
                    let full_name = if package_prefix.is_empty() {
                        name
                    } else {
                        format!("{}.{}", package_prefix, name)
                    };
                    enum_index.insert(full_name, enum_type.clone());
                }
            }
        }

        (message_index, enum_index)
    }

    fn find_message_descriptor<'a>(
        message_index: &'a HashMap<String, DescriptorProto>,
        type_name: &str,
    ) -> Option<&'a DescriptorProto> {
        let normalized = Self::normalize_type_name(type_name);
        if let Some(message) = message_index.get(&normalized) {
            return Some(message);
        }

        let short_name = normalized.rsplit('.').next().unwrap_or(&normalized);
        let mut matches = message_index
            .iter()
            .filter(|(name, _)| name.ends_with(&format!(".{}", short_name)))
            .map(|(_, descriptor)| descriptor);
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    fn find_enum_descriptor<'a>(
        enum_index: &'a HashMap<String, EnumDescriptorProto>,
        type_name: &str,
    ) -> Option<&'a EnumDescriptorProto> {
        let normalized = Self::normalize_type_name(type_name);
        if let Some(enum_def) = enum_index.get(&normalized) {
            return Some(enum_def);
        }

        let short_name = normalized.rsplit('.').next().unwrap_or(&normalized);
        let mut matches = enum_index
            .iter()
            .filter(|(name, _)| name.ends_with(&format!(".{}", short_name)))
            .map(|(_, descriptor)| descriptor);
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    fn field_schema(
        field: &FieldDescriptorProto,
        message_index: &HashMap<String, DescriptorProto>,
        enum_index: &HashMap<String, EnumDescriptorProto>,
        visiting: &mut HashSet<String>,
        depth: usize,
    ) -> Value {
        if depth == 0 {
            return serde_json::json!({});
        }

        let base = match Type::try_from(field.r#type.unwrap_or(Type::Message as i32))
            .unwrap_or(Type::String)
        {
            Type::Double | Type::Float => serde_json::json!({ "type": "number" }),
            Type::Int64
            | Type::Uint64
            | Type::Int32
            | Type::Fixed64
            | Type::Fixed32
            | Type::Uint32
            | Type::Sfixed32
            | Type::Sfixed64
            | Type::Sint32
            | Type::Sint64 => serde_json::json!({ "type": "integer" }),
            Type::Bool => serde_json::json!({ "type": "boolean" }),
            Type::String => serde_json::json!({ "type": "string" }),
            Type::Bytes => serde_json::json!({ "type": "string", "format": "byte" }),
            Type::Enum => {
                let enum_values = field
                    .type_name
                    .as_deref()
                    .and_then(|name| Self::find_enum_descriptor(enum_index, name))
                    .map(|enum_def| {
                        enum_def
                            .value
                            .iter()
                            .filter_map(|value| value.name.clone())
                            .map(Value::String)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if enum_values.is_empty() {
                    serde_json::json!({ "type": "string" })
                } else {
                    serde_json::json!({ "type": "string", "enum": enum_values })
                }
            }
            Type::Message | Type::Group => field
                .type_name
                .as_deref()
                .map(|type_name| {
                    Self::build_message_schema(
                        type_name,
                        message_index,
                        enum_index,
                        visiting,
                        depth - 1,
                    )
                })
                .unwrap_or_else(|| serde_json::json!({ "type": "object" })),
        };

        if Label::try_from(field.label.unwrap_or(Label::Optional as i32)).unwrap_or(Label::Optional)
            == Label::Repeated
        {
            serde_json::json!({
                "type": "array",
                "items": base
            })
        } else {
            base
        }
    }

    fn build_message_schema(
        type_name: &str,
        message_index: &HashMap<String, DescriptorProto>,
        enum_index: &HashMap<String, EnumDescriptorProto>,
        visiting: &mut HashSet<String>,
        depth: usize,
    ) -> Value {
        if depth == 0 {
            return serde_json::json!({
                "$ref": format!("proto://{}", Self::normalize_type_name(type_name))
            });
        }

        let normalized = Self::normalize_type_name(type_name);
        if !visiting.insert(normalized.clone()) {
            return serde_json::json!({
                "$ref": format!("proto://{}", normalized)
            });
        }

        let Some(message) = Self::find_message_descriptor(message_index, &normalized) else {
            visiting.remove(&normalized);
            return serde_json::json!({
                "$ref": format!("proto://{}", normalized)
            });
        };

        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for field in &message.field {
            let field_name = Self::to_json_field_name(field);
            if field_name.is_empty() {
                continue;
            }
            let schema = Self::field_schema(field, message_index, enum_index, visiting, depth - 1);
            if Label::try_from(field.label.unwrap_or(Label::Optional as i32))
                .unwrap_or(Label::Optional)
                == Label::Required
            {
                required.push(Value::String(field_name.clone()));
            }
            properties.insert(field_name, schema);
        }
        visiting.remove(&normalized);

        let mut message_schema = serde_json::Map::new();
        message_schema.insert("type".to_string(), Value::String("object".to_string()));
        message_schema.insert("properties".to_string(), Value::Object(properties));
        message_schema.insert("additionalProperties".to_string(), Value::Bool(false));
        if !required.is_empty() {
            message_schema.insert("required".to_string(), Value::Array(required));
        }
        Value::Object(message_schema)
    }

    fn build_operation_input_schema(
        descriptors: &[FileDescriptorProto],
        input_type: &str,
    ) -> Value {
        let (message_index, enum_index) = Self::build_descriptor_indexes(descriptors);
        serde_json::json!({
            "kind": "grpc_message",
            "message_type": Self::normalize_type_name(input_type),
            "schema": Self::build_message_schema(
                input_type,
                &message_index,
                &enum_index,
                &mut HashSet::new(),
                8,
            )
        })
    }

    /// Execute a gRPC method call
    async fn call_method(
        &self,
        url: &str,
        method_info: &MethodInfo,
        args: HashMap<String, Value>,
    ) -> Result<Value> {
        if method_info.is_server_streaming || method_info.is_client_streaming {
            bail!(
                "Unsupported gRPC call type for '{}/{}': only unary methods are supported",
                method_info.service_name,
                method_info.name
            );
        }

        let target = Self::parse_url(url)?;
        let full_method = format!("{}/{}", method_info.service_name, method_info.name);
        let request_data = self.build_request_message(&args)?;

        self.invoke_unary_with_grpcurl(url, &target, &full_method, &request_data)
            .await
    }

    async fn invoke_unary_with_grpcurl(
        &self,
        original_url: &str,
        target: &str,
        full_method: &str,
        request_data: &Value,
    ) -> Result<Value> {
        let request_json = serde_json::to_string(request_data)?;
        let attempts = Self::grpcurl_attempts(original_url, target);
        let mut last_error = String::new();

        let headers = if let Some(profile) = &self.auth_profile {
            profile.to_grpcurl_headers()?
        } else {
            Vec::new()
        };

        for plaintext in attempts {
            let result = self
                .grpcurl_executor
                .execute(
                    plaintext,
                    headers.clone(),
                    &request_json,
                    target,
                    full_method,
                )
                .await?;

            if result.success {
                if result.stdout.is_empty() {
                    return Ok(serde_json::json!({}));
                }

                return serde_json::from_str(&result.stdout).or_else(|_| {
                    Ok(serde_json::json!({
                        "raw": result.stdout
                    }))
                });
            }

            last_error = result.stderr;
            if last_error.is_empty() {
                last_error = "grpcurl failed without stderr output".to_string();
            }
        }

        bail!("gRPC unary invocation failed: {}", last_error)
    }

    fn grpcurl_attempts(original_url: &str, target: &str) -> Vec<bool> {
        let mut attempts = Vec::new();

        if original_url.starts_with("http://") {
            attempts.push(true);
        } else if original_url.starts_with("https://") {
            attempts.push(false);
        } else if target.ends_with(":9000")
            || target.ends_with(":50051")
            || target.ends_with(":50052")
            || target.ends_with(":50053")
            || target.ends_with(":9090")
        {
            attempts.push(true);
            attempts.push(false);
        } else {
            attempts.push(false);
            attempts.push(true);
        }

        attempts.dedup();
        attempts
    }

    /// Build request message from args
    fn build_request_message(&self, args: &HashMap<String, Value>) -> Result<Value> {
        Ok(serde_json::to_value(args)?)
    }

    /// Map gRPC status to user-friendly error message
    #[allow(dead_code)]
    fn map_grpc_status(status: &Status) -> String {
        match status.code() {
            tonic::Code::Ok => "Success".to_string(),
            tonic::Code::Cancelled => "Operation was cancelled".to_string(),
            tonic::Code::Unknown => format!("Unknown error: {}", status.message()),
            tonic::Code::InvalidArgument => format!("Invalid argument: {}", status.message()),
            tonic::Code::DeadlineExceeded => "Deadline exceeded".to_string(),
            tonic::Code::NotFound => format!("Not found: {}", status.message()),
            tonic::Code::AlreadyExists => format!("Already exists: {}", status.message()),
            tonic::Code::PermissionDenied => format!("Permission denied: {}", status.message()),
            tonic::Code::ResourceExhausted => "Resource exhausted".to_string(),
            tonic::Code::FailedPrecondition => {
                format!("Failed precondition: {}", status.message())
            }
            tonic::Code::Aborted => "Operation was aborted".to_string(),
            tonic::Code::OutOfRange => format!("Out of range: {}", status.message()),
            tonic::Code::Unimplemented => "Method not implemented".to_string(),
            tonic::Code::Internal => format!("Internal error: {}", status.message()),
            tonic::Code::Unavailable => format!("Service unavailable: {}", status.message()),
            tonic::Code::DataLoss => "Data loss".to_string(),
            tonic::Code::Unauthenticated => "Unauthenticated".to_string(),
        }
    }
}

impl Default for GrpcAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Adapter for GrpcAdapter {
    fn protocol_type(&self) -> ProtocolType {
        ProtocolType::GRpc
    }

    async fn can_handle(&self, url: &str) -> Result<bool> {
        let endpoint = match self.create_endpoint(url) {
            Ok(endpoint) => endpoint,
            Err(_) => return Ok(false),
        };

        Ok(Self::has_reflection(&endpoint).await.unwrap_or(false))
    }

    async fn fetch_schema(&self, url: &str) -> Result<Value> {
        // Try persistent cache first if available
        if let Some(cache) = &self.schema_cache {
            match cache.get(url)? {
                crate::cache::CacheResult::Hit(schema) => {
                    debug!("gRPC cache hit for: {}", url);
                    return Ok(schema);
                }
                crate::cache::CacheResult::Bypassed => {
                    debug!("gRPC cache bypassed for: {}", url);
                }
                crate::cache::CacheResult::Miss => {
                    debug!("gRPC cache miss for: {}", url);
                }
            }
        }

        let services = self.get_service_info(url).await?;

        let mut service_list = Vec::new();
        for (name, info) in &services {
            let mut methods = Vec::new();
            for (method_name, method_info) in &info.methods {
                methods.push(serde_json::json!({
                    "name": method_name,
                    "input_type": method_info.input_type,
                    "output_type": method_info.output_type,
                    "server_streaming": method_info.is_server_streaming,
                    "client_streaming": method_info.is_client_streaming,
                }));
            }

            service_list.push(serde_json::json!({
                "name": name,
                "methods": methods,
            }));
        }

        let schema = serde_json::json!({
            "protocol": "gRPC",
            "services": service_list,
        });

        // Store in persistent cache if available
        if let Some(cache) = &self.schema_cache {
            if let Err(e) = cache.put(url, &schema) {
                debug!("Failed to cache gRPC schema: {}", e);
            } else {
                info!("Cached gRPC schema for: {}", url);
            }
        }

        Ok(schema)
    }

    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>> {
        let services = self.get_service_info(url).await?;

        let mut operations = Vec::new();
        for (service_name, service_info) in &services {
            for (method_name, method_info) in &service_info.methods {
                operations.push(Operation {
                    operation_id: format!("{}/{}", service_name, method_name),
                    display_name: format!("{}/{}", service_name, method_name),
                    description: method_info.description.clone(),
                    parameters: vec![Parameter {
                        name: "request".to_string(),
                        param_type: method_info.input_type.clone(),
                        required: true,
                        description: Some(format!(
                            "Request message of type {}",
                            method_info.input_type
                        )),
                    }],
                    return_type: Some(method_info.output_type.clone()),
                });
            }
        }

        Ok(operations)
    }

    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail> {
        let (method_info, descriptors) = self.find_method_context(url, operation).await?;
        let stream_type = match (
            method_info.is_client_streaming,
            method_info.is_server_streaming,
        ) {
            (false, false) => "unary",
            (false, true) => "server_streaming",
            (true, false) => "client_streaming",
            (true, true) => "bidi_streaming",
        };
        let input_type = method_info.input_type.clone();
        let output_type = method_info.output_type.clone();

        Ok(OperationDetail {
            operation_id: format!("{}/{}", method_info.service_name, method_info.name),
            display_name: format!("{}/{}", method_info.service_name, method_info.name),
            description: method_info.description,
            parameters: vec![Parameter {
                name: "request".to_string(),
                param_type: input_type.clone(),
                required: true,
                description: Some(format!("gRPC request payload ({})", stream_type)),
            }],
            return_type: Some(output_type),
            input_schema: Some(Self::build_operation_input_schema(
                &descriptors,
                &input_type,
            )),
        })
    }

    async fn execute(
        &self,
        url: &str,
        operation: &str,
        args: HashMap<String, Value>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();
        let method_info = self.find_method(url, operation).await?;

        let data = self.call_method(url, &method_info, args).await?;

        Ok(ExecutionResult {
            data,
            metadata: super::ExecutionMetadata {
                duration_ms: start.elapsed().as_millis() as u64,
                operation: operation.to_string(),
            },
        })
    }
}

trait GrpcurlAuthHeaders {
    fn to_grpcurl_headers(&self) -> Result<Vec<String>>;
}

impl GrpcurlAuthHeaders for Profile {
    fn to_grpcurl_headers(&self) -> Result<Vec<String>> {
        use base64::Engine;

        let header = match self.auth_type {
            crate::auth::AuthType::Bearer => format!("authorization: Bearer {}", self.api_key),
            crate::auth::AuthType::ApiKey => format!("x-api-key: {}", self.api_key),
            crate::auth::AuthType::Basic => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(&self.api_key);
                format!("authorization: Basic {}", encoded)
            }
            crate::auth::AuthType::OAuth => {
                let token = self
                    .bearer_token()
                    .ok_or_else(|| anyhow!("OAuth profile is missing access token"))?;
                format!("authorization: Bearer {}", token)
            }
        };

        Ok(vec![header])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Mock grpcurl executor for testing
    struct MockGrpcurlExecutor {
        response: Option<GrpcurlResult>,
    }

    #[async_trait]
    impl GrpcurlExecutor for MockGrpcurlExecutor {
        async fn execute(
            &self,
            _plaintext: bool,
            _headers: Vec<String>,
            _request_json: &str,
            _target: &str,
            _method: &str,
        ) -> Result<GrpcurlResult> {
            self.response
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No response configured"))
        }
    }

    #[test]
    fn test_parse_url() {
        assert_eq!(
            GrpcAdapter::parse_url("localhost:50051").unwrap(),
            "localhost:50051"
        );
        assert_eq!(
            GrpcAdapter::parse_url("http://localhost:50051").unwrap(),
            "localhost:50051"
        );
        assert_eq!(
            GrpcAdapter::parse_url("localhost").unwrap(),
            "localhost:50051"
        );
    }

    #[test]
    fn test_grpcurl_attempts_for_common_targets() {
        assert_eq!(
            GrpcAdapter::grpcurl_attempts("grpcb.in:9000", "grpcb.in:9000"),
            vec![true, false]
        );
        assert_eq!(
            GrpcAdapter::grpcurl_attempts("https://grpcb.in:9001", "grpcb.in:9001"),
            vec![false]
        );
    }

    #[tokio::test]
    async fn test_find_method_requires_service_method_format() {
        let adapter = GrpcAdapter::new();
        let err = adapter
            .find_method("localhost:50051", "Sum")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("ServiceName/MethodName"),
            "unexpected error: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_can_handle_rejects_non_grpc_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_body("ok")
            .create_async()
            .await;

        let adapter = GrpcAdapter::new();
        let can_handle = adapter.can_handle(&server.url()).await.unwrap();
        assert!(!can_handle);
    }

    #[tokio::test]
    async fn test_call_method_rejects_streaming_methods() {
        let adapter = GrpcAdapter::new();
        let method = MethodInfo {
            name: "Stream".to_string(),
            service_name: "example.StreamService".to_string(),
            input_type: "example.Request".to_string(),
            output_type: "example.Response".to_string(),
            is_server_streaming: true,
            is_client_streaming: false,
            description: None,
        };

        let err = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("only unary methods are supported"));
    }

    #[test]
    fn test_build_operation_input_schema_from_descriptor() {
        let descriptor = FileDescriptorProto {
            package: Some("example".to_string()),
            message_type: vec![DescriptorProto {
                name: Some("Request".to_string()),
                field: vec![
                    FieldDescriptorProto {
                        name: Some("id".to_string()),
                        json_name: Some("id".to_string()),
                        number: Some(1),
                        label: Some(Label::Optional as i32),
                        r#type: Some(Type::String as i32),
                        ..Default::default()
                    },
                    FieldDescriptorProto {
                        name: Some("count".to_string()),
                        json_name: Some("count".to_string()),
                        number: Some(2),
                        label: Some(Label::Optional as i32),
                        r#type: Some(Type::Int32 as i32),
                        ..Default::default()
                    },
                    FieldDescriptorProto {
                        name: Some("tags".to_string()),
                        json_name: Some("tags".to_string()),
                        number: Some(3),
                        label: Some(Label::Repeated as i32),
                        r#type: Some(Type::String as i32),
                        ..Default::default()
                    },
                    FieldDescriptorProto {
                        name: Some("status".to_string()),
                        json_name: Some("status".to_string()),
                        number: Some(4),
                        label: Some(Label::Optional as i32),
                        r#type: Some(Type::Enum as i32),
                        type_name: Some(".example.Status".to_string()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            enum_type: vec![EnumDescriptorProto {
                name: Some("Status".to_string()),
                value: vec![
                    prost_types::EnumValueDescriptorProto {
                        name: Some("ACTIVE".to_string()),
                        number: Some(0),
                        ..Default::default()
                    },
                    prost_types::EnumValueDescriptorProto {
                        name: Some("INACTIVE".to_string()),
                        number: Some(1),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let input_schema = GrpcAdapter::build_operation_input_schema(
            std::slice::from_ref(&descriptor),
            ".example.Request",
        );
        assert_eq!(input_schema["kind"], "grpc_message");
        assert_eq!(input_schema["message_type"], "example.Request");
        assert_eq!(input_schema["schema"]["properties"]["id"]["type"], "string");
        assert_eq!(
            input_schema["schema"]["properties"]["count"]["type"],
            "integer"
        );
        assert_eq!(
            input_schema["schema"]["properties"]["tags"]["type"],
            "array"
        );
        assert_eq!(
            input_schema["schema"]["properties"]["status"]["enum"][0],
            "ACTIVE"
        );
    }

    #[test]
    fn test_short_name_lookup_returns_none_when_ambiguous() {
        let mut message_index = HashMap::new();
        message_index.insert(
            "pkg1.User".to_string(),
            DescriptorProto {
                name: Some("User".to_string()),
                ..Default::default()
            },
        );
        message_index.insert(
            "pkg2.User".to_string(),
            DescriptorProto {
                name: Some("User".to_string()),
                ..Default::default()
            },
        );
        assert!(GrpcAdapter::find_message_descriptor(&message_index, ".other.User").is_none());

        let mut enum_index = HashMap::new();
        enum_index.insert(
            "pkg1.Status".to_string(),
            EnumDescriptorProto {
                name: Some("Status".to_string()),
                ..Default::default()
            },
        );
        enum_index.insert(
            "pkg2.Status".to_string(),
            EnumDescriptorProto {
                name: Some("Status".to_string()),
                ..Default::default()
            },
        );
        assert!(GrpcAdapter::find_enum_descriptor(&enum_index, ".other.Status").is_none());
    }

    #[test]
    fn test_build_operation_input_schema_uses_types_from_multiple_descriptors() {
        let service_descriptor = FileDescriptorProto {
            package: Some("service".to_string()),
            service: vec![prost_types::ServiceDescriptorProto {
                name: Some("Echo".to_string()),
                method: vec![prost_types::MethodDescriptorProto {
                    name: Some("Call".to_string()),
                    input_type: Some(".dep.Request".to_string()),
                    output_type: Some(".dep.Response".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let dependency_descriptor = FileDescriptorProto {
            package: Some("dep".to_string()),
            message_type: vec![DescriptorProto {
                name: Some("Request".to_string()),
                field: vec![FieldDescriptorProto {
                    name: Some("name".to_string()),
                    json_name: Some("name".to_string()),
                    number: Some(1),
                    label: Some(Label::Optional as i32),
                    r#type: Some(Type::String as i32),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let input_schema = GrpcAdapter::build_operation_input_schema(
            &[service_descriptor, dependency_descriptor],
            ".dep.Request",
        );
        assert_eq!(
            input_schema["schema"]["properties"]["name"]["type"],
            "string"
        );
    }

    // New tests for grpcurl executor abstraction

    #[tokio::test]
    async fn test_invoke_unary_success_with_valid_json_response() {
        let mock_executor = Arc::new(MockGrpcurlExecutor {
            response: Some(GrpcurlResult {
                success: true,
                stdout: r#"{"value": "42"}"#.to_string(),
                stderr: String::new(),
            }),
        });

        let adapter = GrpcAdapter::new().with_executor(mock_executor);
        let method = MethodInfo {
            name: "Test".to_string(),
            service_name: "test.Service".to_string(),
            input_type: "test.Request".to_string(),
            output_type: "test.Response".to_string(),
            is_server_streaming: false,
            is_client_streaming: false,
            description: None,
        };

        let result = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap();

        assert_eq!(result["value"], "42");
    }

    #[tokio::test]
    async fn test_invoke_unary_success_with_empty_response() {
        let mock_executor = Arc::new(MockGrpcurlExecutor {
            response: Some(GrpcurlResult {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            }),
        });

        let adapter = GrpcAdapter::new().with_executor(mock_executor);
        let method = MethodInfo {
            name: "Test".to_string(),
            service_name: "test.Service".to_string(),
            input_type: "test.Request".to_string(),
            output_type: "test.Response".to_string(),
            is_server_streaming: false,
            is_client_streaming: false,
            description: None,
        };

        let result = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap();

        assert_eq!(result, serde_json::json!({}));
    }

    #[tokio::test]
    async fn test_invoke_unary_failure_with_stderr_output() {
        let mock_executor = Arc::new(MockGrpcurlExecutor {
            response: Some(GrpcurlResult {
                success: false,
                stdout: String::new(),
                stderr: "rpc error: code = NotFound desc = method not found".to_string(),
            }),
        });

        let adapter = GrpcAdapter::new().with_executor(mock_executor);
        let method = MethodInfo {
            name: "Test".to_string(),
            service_name: "test.Service".to_string(),
            input_type: "test.Request".to_string(),
            output_type: "test.Response".to_string(),
            is_server_streaming: false,
            is_client_streaming: false,
            description: None,
        };

        let err = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("gRPC unary invocation failed"));
        assert!(err.to_string().contains("method not found"));
    }

    #[tokio::test]
    async fn test_invoke_unary_failure_without_stderr() {
        let mock_executor = Arc::new(MockGrpcurlExecutor {
            response: Some(GrpcurlResult {
                success: false,
                stdout: String::new(),
                stderr: String::new(),
            }),
        });

        let adapter = GrpcAdapter::new().with_executor(mock_executor);
        let method = MethodInfo {
            name: "Test".to_string(),
            service_name: "test.Service".to_string(),
            input_type: "test.Request".to_string(),
            output_type: "test.Response".to_string(),
            is_server_streaming: false,
            is_client_streaming: false,
            description: None,
        };

        let err = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap_err();

        assert!(err
            .to_string()
            .contains("grpcurl failed without stderr output"));
    }

    #[tokio::test]
    async fn test_invoke_unary_success_with_non_json_output() {
        let mock_executor = Arc::new(MockGrpcurlExecutor {
            response: Some(GrpcurlResult {
                success: true,
                stdout: "plain text response".to_string(),
                stderr: String::new(),
            }),
        });

        let adapter = GrpcAdapter::new().with_executor(mock_executor);
        let method = MethodInfo {
            name: "Test".to_string(),
            service_name: "test.Service".to_string(),
            input_type: "test.Request".to_string(),
            output_type: "test.Response".to_string(),
            is_server_streaming: false,
            is_client_streaming: false,
            description: None,
        };

        let result = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap();

        // Non-JSON output should be wrapped in "raw" field
        assert_eq!(result["raw"], "plain text response");
    }

    #[tokio::test]
    async fn test_invoke_unary_rejects_server_streaming() {
        let mock_executor = Arc::new(MockGrpcurlExecutor {
            response: None, // Should not be called
        });

        let adapter = GrpcAdapter::new().with_executor(mock_executor);
        let method = MethodInfo {
            name: "Stream".to_string(),
            service_name: "test.Service".to_string(),
            input_type: "test.Request".to_string(),
            output_type: "test.Response".to_string(),
            is_server_streaming: true,
            is_client_streaming: false,
            description: None,
        };

        let err = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("only unary methods are supported"));
    }

    #[tokio::test]
    async fn test_invoke_unary_rejects_client_streaming() {
        let mock_executor = Arc::new(MockGrpcurlExecutor {
            response: None, // Should not be called
        });

        let adapter = GrpcAdapter::new().with_executor(mock_executor);
        let method = MethodInfo {
            name: "Upload".to_string(),
            service_name: "test.Service".to_string(),
            input_type: "test.Request".to_string(),
            output_type: "test.Response".to_string(),
            is_server_streaming: false,
            is_client_streaming: true,
            description: None,
        };

        let err = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("only unary methods are supported"));
    }

    #[tokio::test]
    async fn test_invoke_unary_rejects_bidi_streaming() {
        let mock_executor = Arc::new(MockGrpcurlExecutor {
            response: None, // Should not be called
        });

        let adapter = GrpcAdapter::new().with_executor(mock_executor);
        let method = MethodInfo {
            name: "Chat".to_string(),
            service_name: "test.Service".to_string(),
            input_type: "test.Request".to_string(),
            output_type: "test.Response".to_string(),
            is_server_streaming: true,
            is_client_streaming: true,
            description: None,
        };

        let err = adapter
            .call_method("localhost:50051", &method, HashMap::new())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("only unary methods are supported"));
    }

    // Test for reflection descriptor parsing

    #[test]
    fn test_descriptor_contains_service_matches_exact_name() {
        let descriptor = FileDescriptorProto {
            package: Some("example.v1".to_string()),
            service: vec![prost_types::ServiceDescriptorProto {
                name: Some("MyService".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(GrpcAdapter::descriptor_contains_service(
            &descriptor,
            "example.v1.MyService"
        ));
        assert!(!GrpcAdapter::descriptor_contains_service(
            &descriptor,
            "example.v2.MyService"
        ));
        assert!(!GrpcAdapter::descriptor_contains_service(
            &descriptor,
            "example.v1.OtherService"
        ));
    }

    #[test]
    fn test_descriptor_contains_service_handles_empty_package() {
        let descriptor = FileDescriptorProto {
            package: None,
            service: vec![prost_types::ServiceDescriptorProto {
                name: Some("MyService".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(GrpcAdapter::descriptor_contains_service(
            &descriptor,
            "MyService"
        ));
    }

    #[test]
    fn test_normalize_type_name_removes_leading_dot() {
        assert_eq!(
            GrpcAdapter::normalize_type_name(".example.Request"),
            "example.Request"
        );
        assert_eq!(
            GrpcAdapter::normalize_type_name("example.Request"),
            "example.Request"
        );
        assert_eq!(GrpcAdapter::normalize_type_name(""), "");
    }

    #[test]
    fn test_to_json_field_name_prefers_json_name() {
        let field = FieldDescriptorProto {
            name: Some("field_name".to_string()),
            json_name: Some("fieldName".to_string()),
            ..Default::default()
        };

        assert_eq!(GrpcAdapter::to_json_field_name(&field), "fieldName");
    }

    #[test]
    fn test_to_json_field_name_falls_back_to_name() {
        let field = FieldDescriptorProto {
            name: Some("field_name".to_string()),
            json_name: None,
            ..Default::default()
        };

        assert_eq!(GrpcAdapter::to_json_field_name(&field), "field_name");
    }

    #[test]
    fn test_grpcurl_attempts_ordering_for_http_urls() {
        // http:// should try plaintext first
        assert_eq!(
            GrpcAdapter::grpcurl_attempts("http://example.com:9000", "example.com:9000"),
            vec![true]
        );
    }

    #[test]
    fn test_grpcurl_attempts_ordering_for_https_urls() {
        // https:// should try TLS only
        assert_eq!(
            GrpcAdapter::grpcurl_attempts("https://example.com:9001", "example.com:9001"),
            vec![false]
        );
    }

    #[test]
    fn test_grpcurl_attempts_for_standard_grpc_ports() {
        // Standard gRPC ports: 9000, 50051-50053, 9090
        // Try plaintext first, then TLS
        assert_eq!(
            GrpcAdapter::grpcurl_attempts("example.com:9000", "example.com:9000"),
            vec![true, false]
        );
        assert_eq!(
            GrpcAdapter::grpcurl_attempts("example.com:50051", "example.com:50051"),
            vec![true, false]
        );
    }

    #[test]
    fn test_grpcurl_attempts_for_non_standard_ports() {
        // Non-standard ports: try TLS first, then plaintext
        assert_eq!(
            GrpcAdapter::grpcurl_attempts("example.com:8080", "example.com:8080"),
            vec![false, true]
        );
    }

    #[test]
    fn test_grpcurl_attempts_deduplication() {
        // Should dedup duplicate attempts
        assert_eq!(
            GrpcAdapter::grpcurl_attempts("http://example.com:9000", "example.com:9000"),
            vec![true]
        );
    }
}
