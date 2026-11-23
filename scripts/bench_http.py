"""HTTP capacity harness for 10K/100K/1000K tenant benchmarks.

Run against a single API node (TSP_CLUSTER_MODE=false) so Raft/Redis overhead
does not hide database/API capacity. Results are debug-build numbers.
"""
import asyncio
import json
import statistics
import sys
import time

import httpx

BASE = "http://127.0.0.1:8090"
USERS = {
    "10K": ("bench10k", "bench-10K", "b10k"),
    "100K": ("bench100k", "bench-100K", "b100k"),
    "1000K": ("bench1m", "bench-1000K", "b1m"),
}


def pct(values, p):
    if not values:
        return 0.0
    values = sorted(values)
    index = min(len(values) - 1, max(0, int(round((p / 100.0) * len(values))) - 1))
    return round(values[index], 1)


async def one(client, method, path, token=None, body=None, timeout=120):
    headers = {"Authorization": f"Bearer {token}"} if token else {}
    started = time.perf_counter()
    try:
        response = await client.request(
            method, BASE + path, headers=headers, json=body, timeout=timeout
        )
        response.raise_for_status()
        return (time.perf_counter() - started) * 1000, None
    except Exception as error:  # noqa: BLE001 - harness reports errors
        return (time.perf_counter() - started) * 1000, str(error)


async def timed(client, method, path, token=None, body=None):
    latency, error = await one(client, method, path, token, body)
    return latency, error


async def run_scale(label, username, org, prefix, concurrency, count):
    async with httpx.AsyncClient() as client:
        login_started = time.perf_counter()
        response = await client.post(
            BASE + "/api/auth/login",
            json={"username": username, "password": "admin123"},
            timeout=30,
        )
        response.raise_for_status()
        token = response.json()["token"]
        login_ms = (time.perf_counter() - login_started) * 1000

        # Cold dashboard: wait out the 2s TTL.
        await asyncio.sleep(2.2)
        cold_ms, cold_error = await timed(client, "GET", "/api/dashboard", token)

        warm = []
        for _ in range(10):
            latency, error = await timed(client, "GET", "/api/dashboard", token)
            if not error:
                warm.append(latency)

        list_latencies = []
        for _ in range(10):
            latency, error = await timed(
                client, "GET", "/api/vehicles?page_size=50", token
            )
            if not error:
                list_latencies.append(latency)

        map_latencies = []
        for _ in range(10):
            latency, error = await timed(client, "GET", "/api/vehicles/map", token)
            if not error:
                map_latencies.append(latency)

        bbox_latencies = []
        for _ in range(5):
            latency, error = await timed(
                client,
                "GET",
                "/api/vehicles/map?bbox=121.1,31.0,121.5,31.4",
                token,
            )
            if not error:
                bbox_latencies.append(latency)

        nearby_latencies = []
        for _ in range(5):
            latency, error = await timed(
                client,
                "GET",
                "/api/vehicles/nearby?lat=31.2&lon=121.2&radius_m=5000",
                token,
            )
            if not error:
                nearby_latencies.append(latency)

        async def burst(path):
            semaphore = asyncio.Semaphore(concurrency)

            async def worker():
                async with semaphore:
                    return await one(client, "GET", path, token)

            started = time.perf_counter()
            results = await asyncio.gather(*(worker() for _ in range(count)))
            elapsed = time.perf_counter() - started
            latencies = [value for value, error in results if not error]
            errors = [error for _, error in results if error]
            return latencies, errors, elapsed

        burst_list, list_errors, list_elapsed = await burst("/api/vehicles?page_size=50")
        burst_map, map_errors, map_elapsed = await burst("/api/vehicles/map")

        telemetry_latencies = []
        telemetry_errors = []
        sample_count = min(count, 20)
        semaphore = asyncio.Semaphore(max(1, concurrency // 2))

        async def telemetry(index):
            async with semaphore:
                vehicle_id = f"{prefix}_{(index % max(1, 10)) + 1:07d}"
                body = {
                    "vehicle_id": vehicle_id,
                    "lat": 31.2 + index * 0.00001,
                    "lon": 121.2 + index * 0.00001,
                    "speed": 30.0,
                    "heading": 90.0,
                    "ignition": True,
                    "soc": 60.0,
                    "ts": f"2026-11-17T06:{index % 60:02d}:{(index * 7) % 60:02d}Z",
                }
                return await one(client, "POST", "/api/telemetry", token, body)

        telemetry_started = time.perf_counter()
        results = await asyncio.gather(*(telemetry(i) for i in range(sample_count)))
        telemetry_elapsed = time.perf_counter() - telemetry_started
        for value, error in results:
            if error:
                telemetry_errors.append(error)
            else:
                telemetry_latencies.append(value)

        return {
            "scale": label,
            "org": org,
            "concurrency": concurrency,
            "requests_per_endpoint": count,
            "login_ms": round(login_ms, 1),
            "dashboard_cold_ms": round(cold_ms, 1),
            "dashboard_cold_error": cold_error,
            "dashboard_warm": {
                "avg": round(statistics.mean(warm), 1) if warm else None,
                "p50": pct(warm, 50),
                "p95": pct(warm, 95),
            },
            "vehicles_list_seq": {
                "p50": pct(list_latencies, 50),
                "p95": pct(list_latencies, 95),
            },
            "vehicles_list_burst": {
                "avg": round(statistics.mean(burst_list), 1) if burst_list else None,
                "p50": pct(burst_list, 50),
                "p95": pct(burst_list, 95),
                "p99": pct(burst_list, 99),
                "errors": len(list_errors),
                "rps": round(len(burst_list) / list_elapsed, 2)
                if burst_list and list_elapsed
                else None,
            },
            "map_seq": {"p50": pct(map_latencies, 50), "p95": pct(map_latencies, 95)},
            "map_burst": {
                "avg": round(statistics.mean(burst_map), 1) if burst_map else None,
                "p50": pct(burst_map, 50),
                "p95": pct(burst_map, 95),
                "errors": len(map_errors),
                "rps": round(len(burst_map) / map_elapsed, 2)
                if burst_map and map_elapsed
                else None,
            },
            "bbox_seq": {"p50": pct(bbox_latencies, 50)},
            "nearby_seq": {"p50": pct(nearby_latencies, 50)},
            "telemetry": {
                "p50": pct(telemetry_latencies, 50),
                "p95": pct(telemetry_latencies, 95),
                "errors": len(telemetry_errors),
                "rps": round(len(telemetry_latencies) / telemetry_elapsed, 2)
                if telemetry_latencies
                else None,
            },
        }


async def main():
    plan = {
        "10K": (20, 100),
        "100K": (10, 50),
        "1000K": (5, 20),
    }
    only = sys.argv[1] if len(sys.argv) > 1 else None
    results = []
    for label, (username, org, prefix) in USERS.items():
        if only and only != label:
            continue
        concurrency, count = plan[label]
        print(f"running {label} ...", flush=True)
        result = await run_scale(label, username, org, prefix, concurrency, count)
        results.append(result)
        print(json.dumps(result, ensure_ascii=False, indent=2), flush=True)
    print("JSON_RESULTS=" + json.dumps(results, ensure_ascii=False))


if __name__ == "__main__":
    asyncio.run(main())
