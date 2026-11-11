#!/usr/bin/env python3
"""Verify persisted benchmark records in the disposable SQLite database."""
import argparse
import sqlite3

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", default="BenchmarkRux/records.sqlite")
    parser.add_argument("--expected", type=int, default=3000)
    args = parser.parse_args()
    if args.expected < 0:
        parser.error("--expected must be nonnegative")
    with sqlite3.connect(args.database) as connection:
        total, unique_ids, benchmark_records = connection.execute(
            """SELECT COUNT(*), COUNT(DISTINCT id),
                      SUM(CASE WHEN json_extract(payload, '$.email') LIKE 'bench-%@example.com' THEN 1 ELSE 0 END)
               FROM records"""
        ).fetchone()
    benchmark_records = benchmark_records or 0
    print(f"Database: {args.database}")
    print(f"Total rows: {total}")
    print(f"Unique IDs: {unique_ids}")
    print(f"Benchmark rows: {benchmark_records}")
    if total != unique_ids or benchmark_records != args.expected:
        raise SystemExit("FAIL: duplicate IDs or unexpected benchmark record count")
    print("PASS: benchmark records persisted with unique IDs")

if __name__ == "__main__":
    main()
