-- Manual display order set by an admin: groups within their parent, devices within their group,
-- sensors within their device. NULL = automatic (natural name order, after all placed items).
ALTER TABLE groups ADD COLUMN position INTEGER CHECK (position IS NULL OR position > 0);
ALTER TABLE devices ADD COLUMN position INTEGER CHECK (position IS NULL OR position > 0);
ALTER TABLE sensors ADD COLUMN position INTEGER CHECK (position IS NULL OR position > 0);
