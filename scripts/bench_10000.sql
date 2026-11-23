
\set ON_ERROR_STOP on
SET synchronous_commit = off;
INSERT INTO users (id, username, password_hash, display_name, role, created_at, org_name)
SELECT 'bench_user_b10k', 'bench10k', password_hash, 'Bench 10K', 'operator', now()::text, 'bench-10K'
FROM users WHERE username = 'admin'
ON CONFLICT (username) DO NOTHING;

INSERT INTO vehicles (id, vin, plate_no, model, color, energy_type, driver_name, driver_phone, org_name, created_at)
SELECT 'b10k_' || lpad(i::text, 7, '0'),
       'VINb10k' || lpad(i::text, 10, '0'),
       'b10k-' || lpad(i::text, 7, '0'),
       'ET5', '白', 'EV', 'bench', '', 'bench-10K', now()::text
FROM generate_series(1, 10000) AS i;

INSERT INTO vehicle_state (vehicle_id, lat, lon, speed, heading, ignition, soc, odometer_km, status, updated_at, geom)
SELECT 'b10k_' || lpad(i::text, 7, '0'),
       31.0 + (i % 1000) * 0.0005,
       121.0 + (i % 1000) * 0.0005,
       20 + (i % 60), (i % 360)::double precision, 1, 60, 100 + i * 0.01,
       'moving', now()::text,
       ST_SetSRID(ST_MakePoint(121.0 + (i % 1000) * 0.0005, 31.0 + (i % 1000) * 0.0005), 4326)
FROM generate_series(1, 10000) AS i;

INSERT INTO terminals (id, imei, phone, iccid, model, firmware, protocol, vehicle_id, status, auth_code, last_heartbeat_at, created_at, bound_at)
SELECT 'term_b10k_' || lpad(i::text, 7, '0'),
       lpad((1000000 + i)::text, 15, '0'),
       '', '', 'JT808', '1.0', 'JT/T 808-2019',
       'b10k_' || lpad(i::text, 7, '0'), 'online', 'BENCH',
       now()::text, now()::text, now()::text
FROM generate_series(1, 10000) AS i;

INSERT INTO telemetry (vehicle_id, ts, lat, lon, speed, heading, ignition, soc, odometer_km, source, geom)
SELECT 'b10k_' || lpad(i::text, 7, '0'),
       to_char(now() - (i % 86400) * interval '1 second', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
       31.0 + (i % 1000) * 0.0005,
       121.0 + (i % 1000) * 0.0005,
       20 + (i % 60), (i % 360)::double precision, 1, 60, 100 + i * 0.01, 'bench',
       ST_SetSRID(ST_MakePoint(121.0 + (i % 1000) * 0.0005, 31.0 + (i % 1000) * 0.0005), 4326)
FROM generate_series(1, 10000) AS i;

ANALYZE vehicles;
ANALYZE vehicle_state;
ANALYZE terminals;
ANALYZE telemetry;
