#!/usr/bin/env python3
"""Reproducible HTTP load smoke test; Python standard library only."""
import argparse
import concurrent.futures
import json
import statistics
import time
import urllib.error
import urllib.request

def request(url, timeout):
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            return response.status, (time.perf_counter() - start) * 1000
    except urllib.error.HTTPError as error:
        return error.code, (time.perf_counter() - start) * 1000
    except Exception:
        return 0, (time.perf_counter() - start) * 1000

def percentile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    return round(ordered[min(len(ordered) - 1, int((len(ordered) - 1) * fraction))], 2)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:3000/api/records?limit=10")
    parser.add_argument("--requests", type=int, default=1000)
    parser.add_argument("--concurrency", type=int, default=32)
    parser.add_argument("--timeout", type=float, default=5)
    args = parser.parse_args()
    if args.requests < 1 or not 1 <= args.concurrency <= 1000 or args.timeout <= 0:
        parser.error("requests >= 1, concurrency 1..1000 and timeout > 0 required")
    start = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as executor:
        results = list(executor.map(lambda _: request(args.url, args.timeout), range(args.requests)))
    elapsed = time.perf_counter() - start
    durations = [duration for _, duration in results]
    codes = {}
    for status, _ in results:
        codes[str(status)] = codes.get(str(status), 0) + 1
    print(json.dumps({
        "requests": args.requests,
        "concurrency": args.concurrency,
        "elapsed_seconds": round(elapsed, 3),
        "throughput_rps": round(args.requests / elapsed, 2),
        "latency_ms": {"p50": percentile(durations, .5), "p95": percentile(durations, .95), "p99": percentile(durations, .99), "mean": round(statistics.mean(durations), 2)},
        "http_status_counts": codes
    }, indent=2))
    if codes.get("0", 0):
        raise SystemExit(1)

if __name__ == "__main__":
    main()
