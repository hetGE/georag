"""Read-only verification of the durable per-file wiki-attempt tracking.

Guards the invariants that would silently corrupt the pending count / resume:
  1. files.wiki_attempted_at column exists.
  2. The SQL eligibility filter and its Python mirror agree on the real corpus
     (they are used in different code paths — get_pending_file_ids uses SQL,
     get_sync_status + ingest_sources use the Python mirror).
  3. get_pending_file_ids length == get_sync_status['wiki_pending_files']
     (the id-list and the displayed count must never disagree).
  4. The deprecated scope-keyed blob stays drained ('[]').

Run:  PYTHONPATH=. venv/bin/python scripts/verify_wiki_attempted.py
Exits non-zero on any failure. Touches no data (no commits).
"""
import sys

from sqlalchemy import inspect

from backend.models.database import SessionLocal, engine
from backend.models.schemas import File, AppSettings
from backend.services import wiki_service
from backend.services.wiki_service import _wiki_attempt_sql_filter, _wiki_attempt_needed

ok = True


def check(name, cond, detail=""):
    global ok
    ok = ok and bool(cond)
    print(f"  {'PASS' if cond else 'FAIL'}  {name}{('  — ' + detail) if detail else ''}")


db = SessionLocal()
try:
    cols = [c["name"] for c in inspect(engine).get_columns("files")]
    check("wiki_attempted_at column exists", "wiki_attempted_at" in cols)

    # SQL filter vs Python mirror agree on the live corpus
    sql_needed = (
        db.query(File.id)
        .filter(File.scan_status == "processed")
        .filter(_wiki_attempt_sql_filter())
        .count()
    )
    rows = (
        db.query(File.wiki_attempted_at, File.processed_at)
        .filter(File.scan_status == "processed")
        .all()
    )
    py_needed = sum(1 for a, p in rows if _wiki_attempt_needed(a, p))
    check("SQL filter == Python mirror", sql_needed == py_needed,
          f"sql={sql_needed} py={py_needed}")

    # id-list length matches the displayed pending count
    sync = wiki_service.get_sync_status(db)
    pend = wiki_service.get_pending_file_ids(db)
    check("get_pending_file_ids == wiki_pending_files",
          len(pend) == sync["wiki_pending_files"],
          f"ids={len(pend)} stat={sync['wiki_pending_files']}")

    # deprecated blob drained
    s = db.query(AppSettings).filter(AppSettings.id == 1).first()
    blob = list(s.scheduled_processed_file_ids or []) if s else []
    check("deprecated blob drained", blob == [], f"len={len(blob)}")

    marked = db.query(File).filter(File.wiki_attempted_at.isnot(None)).count()
    print(f"\n  covered={sync['wiki_covered_files']}  "
          f"pending={sync['wiki_pending_files']}  attempt_marked={marked}")
finally:
    db.close()

print("\nRESULT:", "ALL PASS" if ok else "FAILURES ABOVE")
sys.exit(0 if ok else 1)
