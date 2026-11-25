-- KingbaseES / KGIS smoke checks.
-- Run with: ksql -h HOST -p 54321 -U USER -d DB -f scripts/kingbase-verify.sql

SELECT version();

SELECT extname
FROM pg_extension
WHERE extname IN ('kingbase_spatial', 'postgis');

SELECT COUNT(*) AS telemetry_rows FROM telemetry;

SELECT COUNT(*) FILTER (WHERE geom IS NOT NULL) AS telemetry_with_geom
FROM telemetry;

SELECT COUNT(*) FROM vehicle_state WHERE geom IS NOT NULL;

SELECT indexname
FROM pg_indexes
WHERE tablename IN ('telemetry', 'vehicle_state')
  AND indexname LIKE '%geom%'
ORDER BY indexname;

SELECT version FROM schema_migrations WHERE version LIKE '0007%';
