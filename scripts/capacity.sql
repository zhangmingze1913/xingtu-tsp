INSERT INTO telemetry (vehicle_id, ts, lat, lon, speed, heading, ignition, soc, odometer_km, source, geom)
SELECT 'veh_001',
       to_char(now() - (i || ' seconds')::interval, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
       31.20 + (i % 1000) * 0.0001,
       121.40 + (i % 1000) * 0.0001,
       20 + (i % 60),
       (i % 360)::double precision,
       1, 60, 1000 + i * 0.01, 'capacity',
       ST_SetSRID(ST_MakePoint(121.40 + (i % 1000) * 0.0001, 31.20 + (i % 1000) * 0.0001), 4326)
FROM generate_series(1, 100000) AS i;
ANALYZE telemetry;
