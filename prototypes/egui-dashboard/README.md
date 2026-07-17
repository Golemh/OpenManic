# egui dashboard visual spike

This intentionally uses synthetic data and no database. It tests the main assumptions behind choosing egui:

- custom-painted activity timeline;
- responsive dashboard from runtime widget definitions;
- separate layout-edit mode with widget span changes;
- shared semantic theme tokens;
- Today, Overview, Categories, and Calendar scaffolds;
- Pomodoro state that updates without blocking the UI.

Run it from the repository root:

```powershell
cargo run --manifest-path prototypes/egui-dashboard/Cargo.toml
```

This is not the production shell. Drag reordering, persisted layouts, SQLite, platform tracking, full keyboard navigation, and explicit accessibility nodes belong in the vertical slice after the visual direction is accepted.
