#!/usr/bin/env python3
"""Benchmark concurrent writes against the RUX example CRUD application."""
import argparse
import concurrent.futures
import json
import time
import urllib.error
import urllib.request
import uuid

def write(url, timeout, number, run_id):
    payload = json.dumps({
        "nome": "Benchmark " + str(number),
        "email": f"bench-{run_id}-{number}@example.com",
        "idade": str(20 + number % 60)
    }).encode("utf-8")
    req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"}, method="POST")
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            body = json.load(response)
            return response.status, body.get("id"), (time.perf_counter() - start) * 1000
    except urllib.error.HTTPError as error:
        return error.code, None, (time.perf_counter() - start) * 1000
    except Exception:
        return 0, None, (time.perf_counter() - start) * 1000

def percentile(values, fraction):
    ordered = sorted(values)
    return round(ordered[min(len(ordered) - 1, int((len(ordered) - 1) * fraction))], 2)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:3000/api/records")
    parser.add_argument("--requests", type=int, default=1000)
    parser.add_argument("--concurrency", type=int, default=32)
    parser.add_argument("--timeout", type=float, default=10)
    args = parser.parse_args()
    if args.requests < 1 or not 1 <= args.concurrency <= 1000 or args.timeout <= 0:
        parser.error("requests >= 1, concurrency 1..1000, timeout > 0 required")
    print("WARNING: This benchmark creates persistent records. Run only on a disposable test database.", flush=True)
    run_id = uuid.uuid4().hex[:12]
    start = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        results = list(pool.map(lambda i: write(args.url, args.timeout, i, run_id), range(args.requests)))
    elapsed = time.perf_counter() - start
    statuses = {}
    for status, _, _ in results:
        statuses[str(status)] = statuses.get(str(status), 0) + 1
    ids = [record_id for status, record_id, _ in results if status == 201 and isinstance(record_id, str)]
    latencies = [duration for _, _, duration in results]
    print(json.dumps({
        "run_id": run_id,
        "requests": args.requests,
        "concurrency": args.concurrency,
        "elapsed_seconds": round(elapsed, 3),
        "throughput_rps": round(args.requests / elapsed, 2),
        "latency_ms": {"p50": percentile(latencies, .5), "p95": percentile(latencies, .95), "p99": percentile(latencies, .99)},
        "http_status_counts": statuses,
        "unique_created_ids": len(set(ids)),
        "created_ids_match_successful_responses": len(ids) == len(set(ids)) == statuses.get("201", 0)
    }, indent=2))
    if statuses != {"201": args.requests} or len(ids) != len(set(ids)):
        raise SystemExit(1)

if __name__ == "__main__":
    main()
