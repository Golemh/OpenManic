OpenManic for Windows
=====================

Requirements
------------
- Windows 11, 64-bit (x86-64)
- No installer, account, database server, or language runtime is required.

Run OpenManic
-------------
1. Extract the entire ZIP to a writable folder, such as Documents\OpenManic.
2. Double-click OpenManic.exe.
3. Keep OpenManic.exe in that extracted folder. By default, OpenManic creates an
   OpenManicData folder beside it and stores all tracking data locally there.

Windows may show a SmartScreen warning while builds are unsigned. Verify the
download checksum against the accompanying .sha256 file before running it.

Update OpenManic
----------------
1. Quit OpenManic from its tray menu so the database is fully closed.
2. Keep a copy of the old executable until the new version opens successfully.
3. Replace OpenManic.exe with the newer executable from the release ZIP.
4. Leave OpenManicData in place and start OpenManic again.

Moving or backing up data
-------------------------
- When OpenManic is fully closed, copy the complete OpenManicData folder.
- While it is running, use the backup controls in Settings instead of copying
  the live SQLite files.
- Do not place the data directory on a network share.
