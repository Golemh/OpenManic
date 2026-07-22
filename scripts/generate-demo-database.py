#!/usr/bin/env python3
"""Generate a schema-valid OpenManic demo database (openmanic.sqlite3).

The OpenManic application validates a database when it opens it:
  * the schema_migration ledger must contain versions 1..3, each with an
    FNV-1a-64 checksum of the exact migration SQL text;
  * PRAGMA user_version and store_metadata.schema_version must equal 3;
  * store_metadata.store_id must equal store_identity(<data-root path>),
    an FNV-style hash of the *literal* data-directory path string.

This script reproduces those two algorithms exactly (see
crates/openmanic-storage-sqlite/src/migration.rs::migration_checksum and
crates/openmanic/src/composition.rs::store_identity), applies migrations
0001-0003, and seeds a few days of realistic activity so the timeline is
populated. Because store_id is bound to the data path, the demo must be run
with OPENMANIC_DATA_DIR set to DATA_ROOT below (the launcher does this).

Usage:
    python generate-demo-database.py <repo_root> <output_sqlite_path>
"""
import sqlite3
import sys
import os
import hashlib
from datetime import datetime, timezone, timedelta

# The FIXED data-root path the demo launcher pins via OPENMANIC_DATA_DIR.
# store_id is derived from this exact string, so it must match byte-for-byte.
DATA_ROOT = r"C:\Users\Public\OpenManicDemo"

MASK = (1 << 64) - 1
FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x00000100000001B3


def migration_checksum(sql_bytes: bytes) -> bytes:
    """FNV-1a 64-bit over the raw migration bytes -> little-endian 8 bytes."""
    h = FNV_OFFSET
    for b in sql_bytes:
        h ^= b
        h = (h * FNV_PRIME) & MASK
    return h.to_bytes(8, "little")


def store_identity(path_str: str) -> bytes:
    """Reproduce composition::store_identity -> 16 bytes (big-endian halves)."""
    first = FNV_OFFSET
    second = 0x9E3779B97F4A7C15
    for b in path_str.encode("utf-8"):
        first = ((first ^ b) * FNV_PRIME) & MASK
        second = (((second << 5) | (second >> 59)) & MASK) ^ b  # rotate_left(5)
        second = (second * 0x517CC1B727220A95) & MASK
    return first.to_bytes(8, "big") + second.to_bytes(8, "big")


def us(dt: datetime) -> int:
    """UTC microseconds since epoch."""
    return int(dt.replace(tzinfo=timezone.utc).timestamp() * 1_000_000)


def pid(n: int) -> bytes:
    """Deterministic 16-byte public id."""
    return hashlib.sha256(f"openmanic-demo-{n}".encode()).digest()[:16]


def main():
    repo_root = sys.argv[1]
    out_path = sys.argv[2]
    migrations_dir = os.path.join(
        repo_root, "crates", "openmanic-storage-sqlite", "migrations"
    )
    migration_files = [
        (1, "0001_initial.sql"),
        (2, "0002_schedule_exception_boundary_resolution.sql"),
        (3, "0003_foreground_switch_delay.sql"),
    ]

    if os.path.exists(out_path):
        os.remove(out_path)
    for suffix in ("-wal", "-shm"):
        if os.path.exists(out_path + suffix):
            os.remove(out_path + suffix)

    conn = sqlite3.connect(out_path)
    conn.execute("PRAGMA foreign_keys = ON")
    cur = conn.cursor()

    app_version = "0.1.0"
    now = datetime(2026, 7, 21, 18, 0, 0)  # reference "generation day"
    opened = us(now)

    # --- Apply migrations and record the ledger ------------------------------
    for version, filename in migration_files:
        with open(os.path.join(migrations_dir, filename), "rb") as f:
            raw = f.read()
        cur.executescript(raw.decode("utf-8"))
        cur.execute(
            "INSERT INTO schema_migration(version, checksum, applied_utc_us, app_version)"
            " VALUES (?,?,?,?)",
            (version, migration_checksum(raw), opened, app_version),
        )

    conn.execute("PRAGMA user_version = 3")

    # --- store_metadata (store_id bound to DATA_ROOT) ------------------------
    cur.execute(
        "INSERT INTO store_metadata(singleton_id, store_id, data_revision, schema_version,"
        " created_utc_us, last_opened_app_version, last_clean_shutdown_utc_us)"
        " VALUES (1, ?, ?, 3, ?, ?, ?)",
        (store_identity(DATA_ROOT), 0, opened, app_version, opened),
    )

    # --- Categories ----------------------------------------------------------
    categories = [
        ("Development", "#4C9AFF", 2),
        ("Communication", "#FFAB00", 1),
        ("Browsing", "#8777D9", 0),
        ("Design", "#57D9A3", 2),
    ]
    cat_ids = {}
    for i, (name, color, prod) in enumerate(categories, start=1):
        cur.execute(
            "INSERT INTO category(id, public_id, display_name, color_spec, icon_spec,"
            " description, productivity_class, created_utc_us, updated_utc_us)"
            " VALUES (?,?,?,?,NULL,NULL,?,?,?)",
            (i, pid(100 + i), name, color, prod, opened, opened),
        )
        cat_ids[name] = i

    # --- Applications --------------------------------------------------------
    first_seen = us(now - timedelta(days=3))
    apps = [
        ("Visual Studio Code", "Development"),
        ("Google Chrome", "Browsing"),
        ("Slack", "Communication"),
        ("Figma", "Design"),
        ("Windows Terminal", "Development"),
    ]
    app_ids = {}
    for i, (name, cat) in enumerate(apps, start=1):
        cur.execute(
            "INSERT INTO application(id, public_id, display_name, display_name_override,"
            " category_id, exclusion_policy, first_seen_utc_us, last_seen_utc_us, icon_digest)"
            " VALUES (?,?,?,NULL,?,0,?,?,NULL)",
            (i, pid(200 + i), name, cat_ids[cat], first_seen, opened),
        )
        app_ids[name] = i

    # --- Tracker run ---------------------------------------------------------
    run_start = us(now - timedelta(days=3))
    cur.execute(
        "INSERT INTO tracker_run(id, public_id, started_utc_us, ended_utc_us, clean_end,"
        " platform_session_marker, adapter_version, end_evidence)"
        " VALUES (1, ?, ?, ?, 1, NULL, ?, 0)",
        (pid(300), run_start, opened, "windows-0.1.0"),
    )

    # --- Activity intervals: a plausible workday for the last 3 days ---------
    # state=0 (Active) requires application_id NOT NULL; origin 0 = observed.
    schedule = [
        ("Windows Terminal", 9 * 60, 20),
        ("Visual Studio Code", 9 * 60 + 20, 95),
        ("Google Chrome", 11 * 60, 30),
        ("Slack", 11 * 60 + 35, 25),
        ("Visual Studio Code", 12 * 60 + 5, 55),
        ("Figma", 13 * 60 + 10, 40),
        ("Google Chrome", 13 * 60 + 55, 20),
        ("Visual Studio Code", 14 * 60 + 20, 120),
        ("Slack", 16 * 60 + 25, 20),
        ("Google Chrome", 16 * 60 + 50, 35),
    ]
    titles = {
        "Visual Studio Code": "composition.rs - OpenManic - Visual Studio Code",
        "Google Chrome": "OpenManic issues - GitHub - Google Chrome",
        "Slack": "#openmanic - Slack",
        "Figma": "OpenManic Dashboard - Figma",
        "Windows Terminal": "cargo build - Windows Terminal",
    }
    interval_id = 0
    span_id = 0
    title_id = 0
    title_text_ids = {}
    for day in (3, 2, 1):
        base = now - timedelta(days=day)
        base = base.replace(hour=0, minute=0, second=0, microsecond=0)
        for app_name, start_min, dur_min in schedule:
            start_dt = base + timedelta(minutes=start_min)
            end_dt = start_dt + timedelta(minutes=dur_min)
            interval_id += 1
            cur.execute(
                "INSERT INTO activity_interval(id, tracker_run_id, start_utc_us, end_utc_us,"
                " state, cause, application_id, origin, uncertainty_us, source_revision)"
                " VALUES (?,1,?,?,0,0,?,0,0,0)",
                (interval_id, us(start_dt), us(end_dt), app_ids[app_name]),
            )
            # window title span
            title = titles[app_name]
            if title not in title_text_ids:
                title_id += 1
                cur.execute(
                    "INSERT INTO window_title_text(id, text_hash, title) VALUES (?,?,?)",
                    (title_id, hashlib.sha256(title.encode()).digest()[:16], title),
                )
                title_text_ids[title] = title_id
            span_id += 1
            cur.execute(
                "INSERT INTO window_title_span(id, application_id, tracker_run_id, title_text_id,"
                " start_utc_us, end_utc_us, source_revision) VALUES (?,?,1,?,?,?,0)",
                (span_id, app_ids[app_name], title_text_ids[title], us(start_dt), us(end_dt)),
            )

    conn.commit()

    # --- Self-verification: replicate the app's open-time checks -------------
    verify(conn)
    conn.close()
    print(f"OK  wrote {out_path}")
    print(f"    intervals={interval_id} spans={span_id}")
    print(f"    store_id={store_identity(DATA_ROOT).hex()} for path {DATA_ROOT!r}")


def verify(conn):
    cur = conn.cursor()
    problems = []

    ic = cur.execute("PRAGMA integrity_check").fetchone()[0]
    if ic != "ok":
        problems.append(f"integrity_check: {ic}")

    fk = cur.execute("PRAGMA foreign_key_check").fetchall()
    if fk:
        problems.append(f"foreign_key_check: {fk}")

    uv = cur.execute("PRAGMA user_version").fetchone()[0]
    if uv != 3:
        problems.append(f"user_version={uv} (expected 3)")

    rows = cur.execute(
        "SELECT version FROM schema_migration ORDER BY version"
    ).fetchall()
    versions = [r[0] for r in rows]
    if versions != [1, 2, 3]:
        problems.append(f"ledger versions {versions} (expected [1,2,3])")

    sv = cur.execute(
        "SELECT schema_version FROM store_metadata WHERE singleton_id=1"
    ).fetchone()[0]
    if sv != 3:
        problems.append(f"store_metadata.schema_version={sv} (expected 3)")

    sid = cur.execute(
        "SELECT store_id FROM store_metadata WHERE singleton_id=1"
    ).fetchone()[0]
    if bytes(sid) != store_identity(DATA_ROOT):
        problems.append("store_id mismatch")

    n = cur.execute("SELECT count(*) FROM activity_interval").fetchone()[0]
    if n == 0:
        problems.append("no activity intervals")

    if problems:
        raise SystemExit("VERIFICATION FAILED:\n  " + "\n  ".join(problems))
    print("Self-verification passed (integrity, FKs, ledger, metadata, store_id).")


if __name__ == "__main__":
    main()
