# Phase 4 deferred edge cases

This record captures intentionally deferred Phase 4 details. It is not acceptance evidence and
does not change the canonical requirements in `docs/gui/spec/implementation-plan.md`.

## Calendar local-day resolution

OM-410 receives an already-resolved UTC range for the selected local day. The composition layer
still uses a fixed 24-hour UTC helper for Today navigation and does not yet supply the configured
time zone's daylight-saving-aware local-midnight boundaries to Calendar. OM-411 must not claim
real DST behavior until that primary-owned resolver is wired and tested.

## Paused focus history

Persisted paused focus state retains a start and remaining duration, but not the pause instant.
It cannot produce an exact historical interval without inventing time. The initial Calendar read
adapter must keep such sessions out of historical block geometry until a future model addition
records an authoritative pause boundary or a product decision defines the presentation.

## Calendar route presentation

The first Calendar UI pass may defer dense coincident-block hit ordering, selection recovery after a
deleted activity refresh, overnight continuation language, and the full date-picker/time-zone
conversion path. It must still make distinct source types and immutable loading, empty, and error
states visible. OM-412 owns schedule editing parity; Calendar schedule blocks do not create a
second editor in OM-411.

## Calendar schedule cache warm-up

Calendar schedule editing reuses the accepted Timeline schedule snapshots and command construction.
Until the shared schedule cache has been populated by Today, a Calendar-first visit can display
schedule blocks but cannot yet open their edit/delete controls. The route must surface this as a
recoverable loading limitation rather than inventing a separate schedule cache or editor.

## Saved-view document coverage

The existing versioned `SavedViewDocument` validates and falls back deterministically, but its
complete restoration fields are not yet exposed through a storage/application service. OM-401
must preserve invalid-document fallback and revision conflicts; unsupported future schema
migration policy remains a later compatibility review rather than a silent acceptance path.
