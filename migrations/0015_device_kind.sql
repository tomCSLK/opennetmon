-- What a device is (router, switch, access point, ...); it only chooses the icon in the overview.
-- No CHECK, so new kinds need no table rebuild; unknown values read as 'generic'.
ALTER TABLE devices ADD COLUMN kind TEXT NOT NULL DEFAULT 'generic';
