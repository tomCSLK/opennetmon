-- Ping sensors send several echo requests per poll since 0.5.5 and report minimum, maximum, jitter
-- and packet loss besides the average (the existing channel `rtt`, whose history goes on). Existing
-- ping sensors get the new channels without thresholds; their settings (`sensors.params`) stay empty,
-- which means the default number of packets.
INSERT INTO channels (sensor_id, key, unit)
SELECT s.id, c.key, c.unit
FROM sensors s
CROSS JOIN (
    SELECT 1 AS pos, 'rtt_min' AS key, 'ms' AS unit
    UNION ALL SELECT 2, 'rtt_max', 'ms'
    UNION ALL SELECT 3, 'jitter', 'ms'
    UNION ALL SELECT 4, 'loss', 'percent'
) c
WHERE s.kind = 'ping'
  AND NOT EXISTS (SELECT 1 FROM channels x WHERE x.sensor_id = s.id AND x.key = c.key)
ORDER BY s.id, c.pos;
