-- PostGIS spatial benchmark seed/query.
--
-- Usage:
--   psql -U tsp -d tsp -v rows=100000 -f scripts/postgis-bench.sql
--   psql -U tsp -d tsp -v rows=1000000 -f scripts/postgis-bench.sql
--
-- The script clears telemetry, inserts one benchmark vehicle, loads synthetic
-- points around Shanghai, then runs geography/bbox queries with EXPLAIN.
\set ON_ERROR_STOP on

INSERT INTO vehicles (id, vin, plate_no, model, color, energy_type,
                      driver_name, driver_phone, org_name, created_at)
VALUES ('bench_postgis', 'LBENCHPOSTGIS00001', '沪BENCH01', 'bench', 'white', 'EV',
        'bench', '', 'bench', now()::text)
ON CONFLICT (id) DO NOTHING;

TRUNCATE telemetry RESTART IDENTITY;
SET synchronous_commit = off;

INSERT INTO telemetry (vehicle_id, ts, lat, lon, speed, heading, ignition,
                       soc, odometer_km, source, geom)
SELECT
    'bench_postgis',
    to_char(now() - ((g % 604800) * interval '1 second'), 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
    31.0 + random() * 0.6,
    121.2 + random() * 0.6,
    random() * 120.0,
    random() * 360.0,
    CASE WHEN random() > 0.3 THEN 1 ELSE 0 END,
    random() * 100.0,
    random() * 200000.0,
    'bench',
    ST_SetSRID(ST_MakePoint(121.2 + random() * 0.6, 31.0 + random() * 0.6), 4326)
FROM generate_series(1, :rows) AS g;

SET synchronous_commit = on;
ANALYZE telemetry;

SELECT count(*) AS telemetry_rows,
       count(*) FILTER (WHERE geom IS NOT NULL) AS geom_rows,
       pg_size_pretty(pg_total_relation_size('telemetry')) AS table_size
FROM telemetry;

EXPLAIN (ANALYZE, BUFFERS)
SELECT count(*)
FROM telemetry
WHERE ST_DWithin(
    geom::geography,
    ST_SetSRID(ST_MakePoint(121.4737, 31.2304), 4326)::geography,
    5000
);

EXPLAIN (ANALYZE, BUFFERS)
SELECT count(*)
FROM telemetry
WHERE ST_Intersects(
    geom,
    ST_MakeEnvelope(121.40, 31.20, 121.55, 31.30, 4326)
);
