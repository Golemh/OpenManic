# Phase 5 deferred edge cases

This record keeps intentionally deferred Phase 5 behavior visible while the core customization
contracts are implemented. None of these entries are represented as completed product behavior.

## 1. System-theme observation

Follow System will resolve through the same built-in theme path as Dark and Light. Live operating-
system preference observation, platform notifications, and reduced-motion integration require a
separate platform capability boundary and are deferred for now. The built-in selection remains
persisted and resolves atomically; this waiver concerns live notification only.

## 2. Narrow-window interaction treatment

The required deterministic 12/8/4-column reflow behavior is implemented and tested. The
below-720 logical-pixel scroll-shell affordances and real-device touch behavior remain deferred
until a dedicated compact interaction pass.

## 3. Missing first-party renderer recovery controls

The registry preserves a missing/incompatible widget placement and exposes a recoverable model.
The final visual copy and contextual one-click Remove/Reset affordances are deferred; Edit layout
continues to provide the safe recovery path.
