CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS vehicles (
    id TEXT PRIMARY KEY,
    vin TEXT NOT NULL UNIQUE,
    plate_no TEXT NOT NULL UNIQUE,
    model TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '',
    energy_type TEXT NOT NULL DEFAULT 'EV',
    driver_name TEXT NOT NULL DEFAULT '',
    driver_phone TEXT NOT NULL DEFAULT '',
    org_name TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS vehicle_state (
    vehicle_id TEXT PRIMARY KEY REFERENCES vehicles(id) ON DELETE CASCADE,
    lat DOUBLE PRECISION,
    lon DOUBLE PRECISION,
    speed DOUBLE PRECISION NOT NULL DEFAULT 0,
    heading DOUBLE PRECISION NOT NULL DEFAULT 0,
    ignition INTEGER NOT NULL DEFAULT 0,
    soc DOUBLE PRECISION NOT NULL DEFAULT 0,
    odometer_km DOUBLE PRECISION NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'offline',
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS terminals (
    id TEXT PRIMARY KEY,
    imei TEXT NOT NULL UNIQUE,
    phone TEXT NOT NULL DEFAULT '',
    iccid TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    firmware TEXT NOT NULL DEFAULT '',
    protocol TEXT NOT NULL DEFAULT 'JT/T 808-2019',
    vehicle_id TEXT UNIQUE REFERENCES vehicles(id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'offline',
    auth_code TEXT,
    last_heartbeat_at TEXT,
    created_at TEXT NOT NULL,
    bound_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_terminals_vehicle ON terminals(vehicle_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_terminals_phone
    ON terminals(phone) WHERE phone <> '';

CREATE TABLE IF NOT EXISTS telemetry (
    id BIGSERIAL PRIMARY KEY,
    vehicle_id TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    ts TEXT NOT NULL,
    lat DOUBLE PRECISION NOT NULL,
    lon DOUBLE PRECISION NOT NULL,
    speed DOUBLE PRECISION NOT NULL DEFAULT 0,
    heading DOUBLE PRECISION NOT NULL DEFAULT 0,
    ignition INTEGER NOT NULL DEFAULT 0,
    soc DOUBLE PRECISION NOT NULL DEFAULT 0,
    odometer_km DOUBLE PRECISION NOT NULL DEFAULT 0,
    source TEXT NOT NULL DEFAULT 'api',
    extra_json TEXT
);
CREATE INDEX IF NOT EXISTS idx_telemetry_vehicle_ts ON telemetry(vehicle_id, ts);
CREATE INDEX IF NOT EXISTS idx_telemetry_ts ON telemetry(ts);

CREATE TABLE IF NOT EXISTS trips (
    id TEXT PRIMARY KEY,
    vehicle_id TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    start_ts TEXT NOT NULL,
    end_ts TEXT,
    distance_km DOUBLE PRECISION NOT NULL DEFAULT 0,
    avg_speed DOUBLE PRECISION NOT NULL DEFAULT 0,
    max_speed DOUBLE PRECISION NOT NULL DEFAULT 0,
    start_lat DOUBLE PRECISION NOT NULL,
    start_lon DOUBLE PRECISION NOT NULL,
    end_lat DOUBLE PRECISION,
    end_lon DOUBLE PRECISION,
    status TEXT NOT NULL DEFAULT 'active'
);
CREATE INDEX IF NOT EXISTS idx_trips_vehicle_start ON trips(vehicle_id, start_ts);
CREATE INDEX IF NOT EXISTS idx_trips_start ON trips(start_ts);

CREATE TABLE IF NOT EXISTS geofences (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'circle',
    center_lat DOUBLE PRECISION NOT NULL,
    center_lon DOUBLE PRECISION NOT NULL,
    radius_m DOUBLE PRECISION NOT NULL DEFAULT 800,
    polygon TEXT,
    color TEXT NOT NULL DEFAULT '#22d3ee',
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS geofence_states (
    vehicle_id TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    geofence_id TEXT NOT NULL REFERENCES geofences(id) ON DELETE CASCADE,
    inside INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (vehicle_id, geofence_id)
);

CREATE TABLE IF NOT EXISTS alerts (
    id TEXT PRIMARY KEY,
    vehicle_id TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    alert_type TEXT NOT NULL,
    level TEXT NOT NULL,
    title TEXT NOT NULL,
    message TEXT NOT NULL,
    lat DOUBLE PRECISION,
    lon DOUBLE PRECISION,
    ts TEXT NOT NULL,
    acknowledged INTEGER NOT NULL DEFAULT 0,
    ack_by TEXT,
    ack_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_alerts_ts ON alerts(ts);
CREATE INDEX IF NOT EXISTS idx_alerts_ack ON alerts(acknowledged, ts);
CREATE INDEX IF NOT EXISTS idx_alerts_vehicle ON alerts(vehicle_id, ts);

CREATE TABLE IF NOT EXISTS commands (
    id TEXT PRIMARY KEY,
    vehicle_id TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    command TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    requested_by TEXT NOT NULL,
    payload TEXT,
    result TEXT,
    created_at TEXT NOT NULL,
    sent_at TEXT,
    completed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_commands_created ON commands(created_at);
CREATE INDEX IF NOT EXISTS idx_commands_vehicle ON commands(vehicle_id, created_at);

CREATE TABLE IF NOT EXISTS audit_logs (
    id BIGSERIAL PRIMARY KEY,
    user_id TEXT,
    username TEXT,
    action TEXT NOT NULL,
    target TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_logs(created_at);

CREATE INDEX IF NOT EXISTS idx_vehicles_org ON vehicles(org_name);
CREATE INDEX IF NOT EXISTS idx_terminals_heartbeat ON terminals(status, last_heartbeat_at);
CREATE INDEX IF NOT EXISTS idx_vehicles_org_plate ON vehicles(org_name, plate_no);
