# Initial categories

OpenManic starts an empty category catalog with a small, general-purpose taxonomy. These categories
give the Today timeline useful color and structure without requiring a long setup step.

| Category | Initial application matches |
| --- | --- |
| Development | Terminal, PowerShell, Command Prompt, VS Code, Visual Studio, JetBrains IDEs, GitHub Desktop |
| Communication | Discord, Slack, Microsoft Teams, Zoom |
| Design | Figma, Photoshop, Illustrator, Blender, DaVinci Resolve |
| Entertainment | Spotify, mpv, VLC, Steam, media players |
| Web Browsing | Chrome, Firefox, Microsoft Edge, Brave |
| AI Assistants | ChatGPT, Claude, Gemini desktop applications |
| Productivity | Word, Excel, PowerPoint, Notion, Obsidian |
| Security & Utilities | KeePass, 1Password, File Explorer, Task Manager |

## Bootstrap and assignment behavior

- The eight categories are created only when the stored category catalog is empty. Existing
  categories are never renamed, replaced, or supplemented automatically.
- Recognized applications already present when the empty catalog is initialized receive the
  corresponding category.
- A newly discovered desktop application receives an initial category when its normalized display
  name matches the conservative list above.
- Once an application exists, its stored category is authoritative. OpenManic preserves later user
  changes, including an explicit move back to Uncategorized.
- Applications without a recognized match remain Uncategorized.
- Idle, Away, Paused, Powered off, and similar values are activity states, not categories.

## Browser limitation

Chrome, Firefox, Edge, and Brave are categorized as Web Browsing. A ChatGPT, Claude, or Gemini page
inside a browser remains browser activity unless a future privacy-aware website classification
feature is explicitly enabled. OpenManic does not infer website categories from sensitive window
titles as part of this default taxonomy.
