---
title: Managed poll pagination
description: Configure bounded page-number polling and detect possible gaps in newest-first feeds.
---

# Managed poll pagination

By default, a managed poll source still fetches one response per round. Opt
into `index_pagination` for a newest-first feed with stable item IDs and
one-based page numbers. This setting works with the `item_key` checkpoint
strategy, not cursor-based feeds, and requires an explicit items array on
every non-304 response.

For a GitHub repository events feed, pass the page size as a **JSON integer**
in the source operation arguments and let UXC supply `page`:

```bash
uxc source ensure ingest repo-events https://api.github.com \
  'get:/repos/{owner}/{repo}/events' \
  '{"owner":"OWNER","repo":"REPO","per_page":100}' \
  --mode poll \
  --poll-config '{
    "interval_secs": 60,
    "extract_items_pointer": "",
    "checkpoint_strategy": {
      "type": "item_key",
      "item_key_pointer": "/id",
      "seen_window": 1024
    },
    "initial_items_limit": 0,
    "index_pagination": {
      "page_size_arg": "per_page",
      "page_arg": "page",
      "max_pages": 3
    }
  }'
```

Use the schema and authentication options needed by your endpoint. The page
argument must not also appear in the source operation arguments. `max_pages`
must be between 1 and 20; when `page_arg` is set, `seen_window` (default 1024)
must be at least `page_size * max_pages`. For **single-page monitoring**
without additional requests, omit `page_arg` and set `max_pages` to 1. The
operation must then return its first page by default.

The first successful round of a fresh source reads only page 1.
`initial_items_limit: 0` remembers the page without replaying it. Later rounds
read from page 1 until a complete page overlaps the persisted checkpoint,
returns fewer items than requested, or reaches the page budget. Repeats
within the same round are deduplicated but do not stop the walk. If a page
fails, none of that round's events or checkpoint changes are committed and
the next attempt starts at page 1. The first page owns the `ETag` and
`X-Poll-Interval` handling; a first-page `304` ends the round without new
events.

## Monitoring and migration

Use `uxc source status NAMESPACE SOURCE_KEY` to read `checkpoint.possible_gap_count`,
`checkpoint.last_possible_gap_reason`, and `checkpoint.last_possible_gap_at_unix`.
These fields appear after the first possible gap. The count measures
**completed rounds with a possible gap**, not missing events. A warning of
type `managed_source_possible_gap` is also written to the daemon log:

| Reason | Meaning |
| --- | --- |
| `single_page_full` | A single-page monitor filled its page without reaching a known item. |
| `page_budget_exhausted` | The final allowed page was full, with no known item on any requested page. |
| `feed_tail_without_overlap` | The first page was full, but a later short or empty page ended the walk without reaching a known item. |

Successful rounds still append the items they fetched. A possible gap does
not set `last_error` or create an event or offset hole. Page-number pagination
is not a snapshot: items can move while UXC fetches pages. No setting can
recover items outside the provider's visible window or detect items the
provider never exposed. Keep the page budget within the provider's window
and request limits.

To migrate a downstream-managed source, set the page size explicitly, remove
any fixed page argument, add `index_pagination` to the desired source spec, and
watch its status after redeploying. `source ensure` with a changed spec keeps
the stream and offsets but creates a new run with a fresh checkpoint; plan
the switch accordingly. With `initial_items_limit: 0`, the first page is only
a baseline, so reconcile any cutover backlog separately if you need it.
Restarting an unchanged source preserves its checkpoint and gap count.
