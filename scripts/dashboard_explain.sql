EXPLAIN (ANALYZE, BUFFERS)
SELECT COUNT(*), COALESCE(AVG(s.soc),0), COALESCE(SUM(s.odometer_km),0), COUNT(t.vehicle_id),
       COALESCE(SUM(CASE WHEN t.status='online' THEN 1 ELSE 0 END),0)
FROM vehicles v LEFT JOIN vehicle_state s ON s.vehicle_id=v.id
LEFT JOIN terminals t ON t.vehicle_id=v.id
WHERE v.org_name='bench-1000K';
EXPLAIN (ANALYZE, BUFFERS)
SELECT COALESCE(s.status,'offline'), COUNT(*) FROM vehicles v
LEFT JOIN vehicle_state s ON s.vehicle_id=v.id
WHERE v.org_name='bench-1000K' GROUP BY 1;
