CREATE TABLE attestations (
    installation_id TEXT PRIMARY KEY REFERENCES installations(id) ON DELETE CASCADE,
    key_id TEXT NOT NULL UNIQUE,
    public_key TEXT NOT NULL,
    receipt TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('production', 'development')),
    counter INTEGER NOT NULL DEFAULT 0 CHECK (counter BETWEEN 0 AND 4294967295),
    last_nonce TEXT NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
    control_day INTEGER NOT NULL DEFAULT 0,
    control_count INTEGER NOT NULL DEFAULT 0 CHECK (control_count BETWEEN 0 AND 128),
    retain_until INTEGER NOT NULL
);
CREATE INDEX attestations_retention ON attestations(retain_until);
ALTER TABLE pairings ADD COLUMN revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1));
CREATE INDEX pairings_installation ON pairings(installation_id);
CREATE INDEX routes_pairing ON routes(pairing_id);
CREATE UNIQUE INDEX routes_app ON routes(pairing_id) WHERE kind = 'app';
CREATE INDEX routes_expiry ON routes(expires_at);
CREATE TABLE control_commits (
    installation_id TEXT PRIMARY KEY REFERENCES installations(id) ON DELETE CASCADE,
    key_id TEXT NOT NULL,
    previous_counter INTEGER NOT NULL,
    counter INTEGER NOT NULL,
    epoch INTEGER NOT NULL,
    nonce TEXT NOT NULL,
    now INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    terminal_revocation INTEGER NOT NULL DEFAULT 0 CHECK (terminal_revocation IN (0, 1))
);
CREATE TABLE control_usage (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    day INTEGER NOT NULL,
    controls INTEGER NOT NULL CHECK (controls BETWEEN 0 AND 1000),
    enrollments INTEGER NOT NULL CHECK (enrollments BETWEEN 0 AND 30)
);
INSERT INTO control_usage VALUES (1, 0, 0, 0);
CREATE TRIGGER enrollment_limit BEFORE INSERT ON attestations
BEGIN
    UPDATE control_usage SET enrollments = CASE WHEN day = CAST(unixepoch() / 86400 AS INTEGER) THEN enrollments ELSE 0 END + 1,
        controls = CASE WHEN day = CAST(unixepoch() / 86400 AS INTEGER) THEN controls ELSE 0 END,
        day = CAST(unixepoch() / 86400 AS INTEGER)
    WHERE id = 1 AND (day <> CAST(unixepoch() / 86400 AS INTEGER) OR enrollments < 30);
    SELECT CASE WHEN changes() <> 1 THEN RAISE(ABORT, 'control_quota') END;
END;
CREATE TRIGGER control_commit BEFORE INSERT ON control_commits
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM attestations a JOIN installations i ON i.id = a.installation_id
        WHERE a.installation_id = NEW.installation_id AND a.key_id = NEW.key_id
          AND a.counter = NEW.previous_counter AND NEW.counter > a.counter AND (a.revoked = 0 OR NEW.terminal_revocation = 1)
          AND i.epoch = NEW.epoch AND a.last_nonce <> NEW.nonce
          AND NEW.expires_at > MAX(NEW.now, unixepoch())
    ) THEN RAISE(ABORT, 'unauthorized_control') END;
    UPDATE attestations SET counter = NEW.counter, last_nonce = NEW.nonce,
        control_count = CASE WHEN control_day = CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER) THEN control_count ELSE 0 END + 1,
        control_day = CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER)
    WHERE installation_id = NEW.installation_id AND control_day <= CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER)
        AND (control_day <> CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER) OR control_count < 128);
    SELECT CASE WHEN changes() <> 1 THEN RAISE(ABORT, 'control_quota') END;
    UPDATE control_usage SET controls = CASE WHEN day = CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER) THEN controls ELSE 0 END + 1,
        enrollments = CASE WHEN day = CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER) THEN enrollments ELSE 0 END,
        day = CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER)
    WHERE id = 1 AND day <= CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER)
        AND (day <> CAST(MAX(NEW.now, unixepoch()) / 86400 AS INTEGER) OR controls < 1000);
    SELECT CASE WHEN changes() <> 1 THEN RAISE(ABORT, 'control_quota') END;
END;
CREATE TRIGGER route_limit BEFORE INSERT ON routes
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM pairings p JOIN attestations a ON a.installation_id = p.installation_id
        WHERE p.id = NEW.pairing_id AND p.revoked = 0 AND a.revoked = 0
    ) THEN RAISE(ABORT, 'revoked_pairing') END;
    SELECT CASE WHEN (SELECT COUNT(*) FROM routes r JOIN pairings p ON p.id = r.pairing_id
        WHERE p.installation_id = (SELECT installation_id FROM pairings WHERE id = NEW.pairing_id)) >= 128
        THEN RAISE(ABORT, 'route_limit') END;
    SELECT CASE WHEN NEW.kind = 'liveactivity'  AND (
        SELECT COUNT(*) FROM routes r JOIN pairings p ON p.id = r.pairing_id
        WHERE p.installation_id = (SELECT installation_id FROM pairings WHERE id = NEW.pairing_id)
          AND p.revoked = 0 AND r.kind = 'liveactivity' AND r.revoked = 0 AND r.expires_at > unixepoch()
    ) >= 8 THEN RAISE(ABORT, 'activity_limit') END;
END;
CREATE TRIGGER pairing_limit BEFORE INSERT ON pairings
BEGIN
    SELECT CASE WHEN (SELECT COUNT(*) FROM pairings WHERE installation_id = NEW.installation_id) >= 64
        THEN RAISE(ABORT, 'pairing_limit') END;
    SELECT CASE WHEN (SELECT COUNT(*) FROM pairings WHERE installation_id = NEW.installation_id AND revoked = 0) >= 8
    THEN RAISE(ABORT, 'pairing_limit') END;
END;
