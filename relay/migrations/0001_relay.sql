PRAGMA foreign_keys = ON;

CREATE TABLE installations (
    id TEXT PRIMARY KEY,
    epoch INTEGER NOT NULL DEFAULT 0 CHECK (epoch >= 0)
);
CREATE TABLE pairings (
    id TEXT PRIMARY KEY,
    installation_id TEXT NOT NULL REFERENCES installations(id),
    epoch INTEGER NOT NULL DEFAULT 0 CHECK (epoch >= 0)
);
CREATE TABLE routes (
    id TEXT PRIMARY KEY,
    pairing_id TEXT NOT NULL REFERENCES pairings(id),
    generation INTEGER NOT NULL CHECK (generation >= 1),
    destination TEXT NOT NULL,
    destination_digest TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('app', 'liveactivity')),
    environment TEXT NOT NULL CHECK (environment IN ('production', 'development')),
    expires_at INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1))
);
CREATE TABLE global_usage (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    day INTEGER NOT NULL,
    total INTEGER NOT NULL CHECK (total BETWEEN 0 AND 20000),
    routine INTEGER NOT NULL CHECK (routine BETWEEN 0 AND 16000)
);
INSERT INTO global_usage VALUES (1, 0, 0, 0);
CREATE TABLE installation_usage (
    installation_id TEXT PRIMARY KEY REFERENCES installations(id),
    day INTEGER NOT NULL,
    total INTEGER NOT NULL CHECK (total BETWEEN 0 AND 1000),
    routine INTEGER NOT NULL CHECK (routine BETWEEN 0 AND 700)
);

CREATE TRIGGER quota_update BEFORE UPDATE ON installation_usage
BEGIN
    UPDATE global_usage SET
        total = CASE WHEN day = NEW.day THEN total ELSE 0 END + 1,
        routine = CASE WHEN day = NEW.day THEN routine ELSE 0 END
            + NEW.routine - CASE WHEN OLD.day = NEW.day THEN OLD.routine ELSE 0 END,
        day = NEW.day
    WHERE id = 1 AND day <= NEW.day
      AND (day <> NEW.day OR total < 20000)
      AND (day <> NEW.day OR routine + NEW.routine - CASE WHEN OLD.day = NEW.day THEN OLD.routine ELSE 0 END <= 16000);
    SELECT CASE WHEN changes() <> 1 THEN RAISE(ABORT, 'daily_quota') END;
END;
