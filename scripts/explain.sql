VACUUM (ANALYZE) telemetry;
EXPLAIN (ANALYZE, BUFFERS)
SELECT id, ts, lat, lon, speed, heading, ignition, soc, odometer_km, source
FROM (
  SELECT id, ts, lat, lon, speed, heading, ignition, soc, odometer_km, source
  FROM telemetry
  WHERE vehicle_id = 'veh_001'
    AND ts >= to_char(now() - interval '7 days', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
    AND ts <= to_char(now(), 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
  ORDER BY ts DESC LIMIT 1000
) AS recent ORDER BY ts ASC;
