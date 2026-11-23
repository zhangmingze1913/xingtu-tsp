CREATE TABLE IF NOT EXISTS users (
    id VARCHAR(64) NOT NULL,
    username VARCHAR(64) NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    display_name VARCHAR(128) NOT NULL,
    role VARCHAR(32) NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE KEY idx_users_username (username)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS vehicles (
    id VARCHAR(64) NOT NULL,
    vin VARCHAR(32) NOT NULL,
    plate_no VARCHAR(32) NOT NULL,
    model VARCHAR(128) NOT NULL,
    color VARCHAR(32) NOT NULL DEFAULT '',
    energy_type VARCHAR(16) NOT NULL DEFAULT 'EV',
    driver_name VARCHAR(128) NOT NULL DEFAULT '',
    driver_phone VARCHAR(32) NOT NULL DEFAULT '',
    org_name VARCHAR(128) NOT NULL DEFAULT '',
    created_at VARCHAR(40) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE KEY idx_vehicles_vin (vin),
    UNIQUE KEY idx_vehicles_plate (plate_no)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS vehicle_state (
    vehicle_id VARCHAR(64) NOT NULL,
    lat DOUBLE,
    lon DOUBLE,
    speed DOUBLE NOT NULL DEFAULT 0,
    heading DOUBLE NOT NULL DEFAULT 0,
    ignition INTEGER NOT NULL DEFAULT 0,
    soc DOUBLE NOT NULL DEFAULT 0,
    odometer_km DOUBLE NOT NULL DEFAULT 0,
    status VARCHAR(32) NOT NULL DEFAULT 'offline',
    updated_at VARCHAR(40) NOT NULL,
    PRIMARY KEY (vehicle_id),
    CONSTRAINT fk_vehicle_state_vehicle FOREIGN KEY (vehicle_id)
        REFERENCES vehicles (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS terminals (
    id VARCHAR(64) NOT NULL,
    imei VARCHAR(32) NOT NULL,
    phone VARCHAR(32) NOT NULL DEFAULT '',
    iccid VARCHAR(64) NOT NULL DEFAULT '',
    model VARCHAR(64) NOT NULL DEFAULT '',
    firmware VARCHAR(64) NOT NULL DEFAULT '',
    protocol VARCHAR(64) NOT NULL DEFAULT 'JT/T 808-2019',
    vehicle_id VARCHAR(64) DEFAULT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'offline',
    auth_code VARCHAR(64) DEFAULT NULL,
    last_heartbeat_at VARCHAR(40) DEFAULT NULL,
    created_at VARCHAR(40) NOT NULL,
    bound_at VARCHAR(40) DEFAULT NULL,
    phone_key VARCHAR(32) GENERATED ALWAYS AS (NULLIF(phone, '')) STORED,
    PRIMARY KEY (id),
    UNIQUE KEY idx_terminals_imei (imei),
    UNIQUE KEY idx_terminals_phone (phone_key),
    UNIQUE KEY idx_terminals_vehicle (vehicle_id),
    CONSTRAINT fk_terminals_vehicle FOREIGN KEY (vehicle_id)
        REFERENCES vehicles (id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS telemetry (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    vehicle_id VARCHAR(64) NOT NULL,
    ts VARCHAR(40) NOT NULL,
    lat DOUBLE NOT NULL,
    lon DOUBLE NOT NULL,
    speed DOUBLE NOT NULL DEFAULT 0,
    heading DOUBLE NOT NULL DEFAULT 0,
    ignition INTEGER NOT NULL DEFAULT 0,
    soc DOUBLE NOT NULL DEFAULT 0,
    odometer_km DOUBLE NOT NULL DEFAULT 0,
    source VARCHAR(32) NOT NULL DEFAULT 'api',
    extra_json TEXT,
    KEY idx_telemetry_vehicle_ts (vehicle_id, ts),
    KEY idx_telemetry_ts (ts),
    CONSTRAINT fk_telemetry_vehicle FOREIGN KEY (vehicle_id)
        REFERENCES vehicles (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS trips (
    id VARCHAR(64) NOT NULL,
    vehicle_id VARCHAR(64) NOT NULL,
    start_ts VARCHAR(40) NOT NULL,
    end_ts VARCHAR(40) DEFAULT NULL,
    distance_km DOUBLE NOT NULL DEFAULT 0,
    avg_speed DOUBLE NOT NULL DEFAULT 0,
    max_speed DOUBLE NOT NULL DEFAULT 0,
    start_lat DOUBLE NOT NULL,
    start_lon DOUBLE NOT NULL,
    end_lat DOUBLE DEFAULT NULL,
    end_lon DOUBLE DEFAULT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'active',
    PRIMARY KEY (id),
    KEY idx_trips_vehicle_start (vehicle_id, start_ts),
    KEY idx_trips_start (start_ts),
    CONSTRAINT fk_trips_vehicle FOREIGN KEY (vehicle_id)
        REFERENCES vehicles (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS geofences (
    id VARCHAR(64) NOT NULL,
    name VARCHAR(128) NOT NULL,
    kind VARCHAR(16) NOT NULL DEFAULT 'circle',
    center_lat DOUBLE NOT NULL,
    center_lon DOUBLE NOT NULL,
    radius_m DOUBLE NOT NULL DEFAULT 800,
    polygon VARCHAR(8000) DEFAULT NULL,
    color VARCHAR(32) NOT NULL DEFAULT '#22d3ee',
    active INTEGER NOT NULL DEFAULT 1,
    created_at VARCHAR(40) NOT NULL,
    PRIMARY KEY (id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS geofence_states (
    vehicle_id VARCHAR(64) NOT NULL,
    geofence_id VARCHAR(64) NOT NULL,
    inside INTEGER NOT NULL,
    updated_at VARCHAR(40) NOT NULL,
    PRIMARY KEY (vehicle_id, geofence_id),
    KEY idx_geofence_states_fence (geofence_id),
    CONSTRAINT fk_geofence_states_vehicle FOREIGN KEY (vehicle_id)
        REFERENCES vehicles (id) ON DELETE CASCADE,
    CONSTRAINT fk_geofence_states_fence FOREIGN KEY (geofence_id)
        REFERENCES geofences (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS alerts (
    id VARCHAR(64) NOT NULL,
    vehicle_id VARCHAR(64) NOT NULL,
    alert_type VARCHAR(32) NOT NULL,
    level VARCHAR(16) NOT NULL,
    title VARCHAR(255) NOT NULL,
    message VARCHAR(2048) NOT NULL,
    lat DOUBLE DEFAULT NULL,
    lon DOUBLE DEFAULT NULL,
    ts VARCHAR(40) NOT NULL,
    acknowledged INTEGER NOT NULL DEFAULT 0,
    ack_by VARCHAR(64) DEFAULT NULL,
    ack_at VARCHAR(40) DEFAULT NULL,
    PRIMARY KEY (id),
    KEY idx_alerts_ts (ts),
    KEY idx_alerts_ack (acknowledged, ts),
    KEY idx_alerts_vehicle (vehicle_id, ts),
    CONSTRAINT fk_alerts_vehicle FOREIGN KEY (vehicle_id)
        REFERENCES vehicles (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS commands (
    id VARCHAR(64) NOT NULL,
    vehicle_id VARCHAR(64) NOT NULL,
    command VARCHAR(32) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'pending',
    requested_by VARCHAR(64) NOT NULL,
    payload VARCHAR(8000) DEFAULT NULL,
    result VARCHAR(2048) DEFAULT NULL,
    created_at VARCHAR(40) NOT NULL,
    sent_at VARCHAR(40) DEFAULT NULL,
    completed_at VARCHAR(40) DEFAULT NULL,
    PRIMARY KEY (id),
    KEY idx_commands_created (created_at),
    KEY idx_commands_vehicle (vehicle_id, created_at),
    CONSTRAINT fk_commands_vehicle FOREIGN KEY (vehicle_id)
        REFERENCES vehicles (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS audit_logs (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id VARCHAR(64) DEFAULT NULL,
    username VARCHAR(64) DEFAULT NULL,
    action VARCHAR(64) NOT NULL,
    target VARCHAR(64) NOT NULL,
    detail VARCHAR(2048) NOT NULL DEFAULT '',
    created_at VARCHAR(40) NOT NULL,
    KEY idx_audit_created (created_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_vehicles_org ON vehicles(org_name);
CREATE INDEX idx_terminals_heartbeat ON terminals(status, last_heartbeat_at);
CREATE INDEX idx_vehicles_org_plate ON vehicles(org_name, plate_no);
