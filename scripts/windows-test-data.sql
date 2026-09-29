-- Execute in a dedicated development database with PostGIS installed.
-- CREATE SCHEMA uses a fresh name so existing test data stays protected.
BEGIN;
CREATE SCHEMA geoledger_demo;
CREATE TABLE geoledger_demo.roads (
    id bigint PRIMARY KEY,
    name text,
    width numeric,
    geom geometry(LineString, 4326)
);
INSERT INTO geoledger_demo.roads VALUES
    (1, 'road-1', 6.50, ST_GeomFromText('LINESTRING(0 0,1 1)', 4326)),
    (2, 'road-2', 8.25, ST_GeomFromText('LINESTRING(1 1,2 1)', 4326));
COMMIT;
