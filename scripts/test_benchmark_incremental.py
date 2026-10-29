#!/usr/bin/env python3
import json
import math
import os
import re
import sqlite3
from pathlib import Path
import tempfile
import unittest

from benchmark_incremental import (collection, rows, self_test_source_crud, check_results,
    compact, fixture_crs, postgis_srid, postgis_seed_sql, shift_geometry, sqlite_seed,
    check_storage_format, STORAGE_FORMAT, INTERNAL_SRID)


class FixtureTests(unittest.TestCase):
    def test_source_batches_detect_partial_writes_and_corruption(self):
        self_test_source_crud()

    def test_multipart_xyz_conversion_preserves_rings_and_z(self):
        ring = [[1.0, 2.0, 3.0], [2.0, 3.0, 4.0], [3.0, 2.0, 5.0], [1.0, 2.0, 3.0]]
        geometry = {"type": "MultiPolygon", "coordinates": [[ring]]}
        converted = json.loads(collection(geometry))
        self.assertEqual(converted["geometries"], [{"type": "Polygon", "coordinates": [ring]}])
        self.assertTrue(collection(geometry).startswith('{"type":"GeometryCollection"'))

    def test_fixture_ids_and_attributes_are_checked_incrementally(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.jsonl"
            feature = {"type": "Feature", "id": "000000000001", "properties": {"attributes": {"n": 18446744073709551615}}, "geometry": {"type": "Point", "coordinates": [1.0, 2.0]}}
            path.write_text(json.dumps(feature) + "\n")
            self.assertEqual(list(rows(path)), [feature])
            path.write_text(json.dumps(feature) + "\n" + json.dumps(feature) + "\n")
            iterator = rows(path)
            self.assertEqual(next(iterator), feature)
            with self.assertRaisesRegex(ValueError, "row 2"):
                next(iterator)


class CoordinateContractTests(unittest.TestCase):
    def test_crs_is_explicit_and_conflicting_metadata_is_rejected(self):
        self.assertEqual(fixture_crs({"crs": " EPSG:3857 "}), "EPSG:3857")
        self.assertEqual(fixture_crs({}, "EPSG:4326"), "EPSG:4326")
        self.assertEqual(fixture_crs({"crs": "EPSG:4490"}, "EPSG:4490"), "EPSG:4490")
        for metadata, requested in [({}, None), ({"crs": "EPSG:3857"}, "EPSG:4326"),
                                     ({"crs": ""}, None), ({"crs": 4326}, None),
                                     ({"crs": "x\0y"}, None)]:
            with self.subTest(metadata=metadata, requested=requested), self.assertRaises(ValueError):
                fixture_crs(metadata, requested)
        self.assertEqual(fixture_crs({"crs": "+proj=tmerc +units=m"}), "+proj=tmerc +units=m")

    def test_business_srid_is_preserved_without_a_second_current_copy(self):
        for crs, expected in [("EPSG:4326", 4326), ("EPSG:3857", 3857), ("POSTGIS:0", 0)]:
            self.assertEqual(postgis_srid(crs), expected)
            sql = postgis_seed_sql("00000000-0000-4000-8000-000000000001", "00000000-0000-4000-8000-000000000002", 1, expected)
            source = next(line for line in sql.splitlines() if line.startswith("INSERT INTO business.source"))
            self.assertIn(f"->'geometry'),{expected})", source)
            self.assertNotIn("INSERT INTO gl_history", sql)
            self.assertNotIn("ST_Transform", sql)
            self.assertIn(r"DELIMITER E'\x01'", sql)
            self.assertIn("PERFORM project,dataset,revision,definition FROM gl_schema_history LIMIT 0", sql)
            self.assertIn("PERFORM project,workspace,dataset,definition FROM gl_workspace_schemas LIMIT 0", sql)
            self.assertIn("fixture seeding rejects existing schema state", sql)
            self.assertIn("DELETE FROM gl_source_changes WHERE schema_name='business' AND table_name='source';", sql)
            self.assertNotIn("DELETE FROM gl_source_changes;", sql)
            self.assertNotIn("DELETE FROM gl_schema_history", sql)
            self.assertNotIn("DELETE FROM gl_workspace_schemas", sql)
        for crs in ["EPSG:-1", "EPSG:3857);DROP TABLE x", "+proj=tmerc", "EPSG:9999999999"]:
            with self.subTest(crs=crs), self.assertRaises(ValueError):
                postgis_srid(crs)

    def test_seed_contract_tracks_current_engine_format(self):
        engine = (Path(__file__).resolve().parents[1] / "crates/engine/src/lib.rs").read_text()
        self.assertIn(f"pub const FORMAT_VERSION: i32 = {STORAGE_FORMAT};", engine)
        check_storage_format(STORAGE_FORMAT)
        for version in [STORAGE_FORMAT - 1, STORAGE_FORMAT + 1]:
            with self.subTest(version=version), self.assertRaisesRegex(ValueError, "rebuild the current server"):
                check_storage_format(version)

    def test_native_translation_preserves_type_z_and_ring_closure(self):
        ring = [[500000.0, 3500000.0, 3], [500001.0, 3500001.0, 4],
                [500002.0, 3500000.0, 5], [500000.0, 3500000.0, 3]]
        geometry = {"type": "MultiPolygon", "coordinates": [[ring]]}
        original = json.loads(compact(geometry))
        shift_geometry(geometry)
        self.assertEqual(geometry["type"], "MultiPolygon")
        self.assertEqual(ring[0], ring[-1])
        self.assertEqual([v[2] for v in ring], [3, 4, 5, 3])
        self.assertNotEqual(geometry["coordinates"], original["coordinates"])
        for xy in [[181.0, -91.0], [1e15, -1e15]]:
            geometry = {"type": "Point", "coordinates": xy + [9]}
            shift_geometry(geometry)
            self.assertTrue(all(math.isfinite(n) for n in geometry["coordinates"]))
            self.assertNotEqual(geometry["coordinates"][0], xy[0])
            self.assertNotEqual(geometry["coordinates"][1], xy[1])
            self.assertEqual(geometry["coordinates"][2], 9)
        for coordinates in [[], [True, 1], [float("inf"), 1], [[1, 2], [3]]]:
            with self.subTest(coordinates=coordinates), self.assertRaises(ValueError):
                shift_geometry({"type": "LineString", "coordinates": coordinates})


@unittest.skipUnless(os.environ.get("GL_SPATIALITE_EXTENSION"), "requires trusted SpatiaLite extension")
class SpatialiteSeedTests(unittest.TestCase):
    def connect(self, path):
        db = sqlite3.connect(path)
        db.enable_load_extension(True)
        db.load_extension(os.environ["GL_SPATIALITE_EXTENSION"])
        db.enable_load_extension(False)
        return db

    def initialize(self, path, srid=0, version=STORAGE_FORMAT):
        db = self.connect(path)
        db.execute("SELECT InitSpatialMetaData(1, 'WGS84')")
        db.execute("CREATE TABLE gl_format(version INTEGER)")
        db.execute("INSERT INTO gl_format VALUES(?)", (version,))
        table = "gl_data_" + "0" * 32
        db.execute("CREATE TABLE gl_datasets(project TEXT,id TEXT,storage_table TEXT,PRIMARY KEY(project,id))")
        db.execute("INSERT INTO gl_datasets VALUES('p','d',?)", (table,))
        db.execute("CREATE TABLE gl_workspaces(project TEXT,id TEXT,PRIMARY KEY(project,id))")
        db.execute("INSERT INTO gl_workspaces VALUES('p','w')")
        db.execute("CREATE TABLE gl_purge(project TEXT PRIMARY KEY)")
        engine_sql = (Path(__file__).resolve().parents[1] / "crates/engine/src/sqlite.sql").read_text()
        for name in ["gl_schema_history", "gl_workspace_schemas"]:
            ddl = re.search(rf"CREATE TABLE {name} \([\s\S]*?\);", engine_sql)
            self.assertIsNotNone(ddl, name)
            db.executescript(ddl.group(0))
        for ddl in engine_sql.splitlines():
            if ddl.startswith("CREATE TRIGGER gl_schema_history_"):
                db.executescript(ddl)
        db.execute(f"CREATE TABLE {table}(feature_id TEXT PRIMARY KEY,valid_from INTEGER NOT NULL,properties TEXT,geometry_json TEXT,geom BLOB,geom_z BLOB)")
        for column, dimension in [("geom", "XY"), ("geom_z", "XYZ")]:
            self.assertEqual(db.execute("SELECT RecoverGeometryColumn(?,?,?,'GEOMETRY',?)", (table, column, srid, dimension)).fetchone()[0], 1)
        db.commit()
        return db

    @staticmethod
    def feature(index, geometry):
        return {"type": "Feature", "id": f"{index:012}", "properties": {"attributes": {"n": 18446744073709551615}}, "geometry": geometry}

    def test_xy_and_xyz_point_line_polygon_seed_matches_service_contract(self):
        for dimension in (2, 3):
            def point(x, y):
                return [x, y] + ([7.0] if dimension == 3 else [])
            ring = [point(500000, 3500000), point(500010, 3500010), point(500020, 3500000), point(500000, 3500000)]
            geometries = [
                {"type": "Point", "coordinates": point(500000, 3500000)},
                {"type": "MultiLineString", "coordinates": [[point(500000, 3500000), point(500001, 3500001)]]},
                {"type": "MultiPolygon", "coordinates": [[ring]]},
            ]
            for geometry in geometries:
                with self.subTest(dimension=dimension, kind=geometry["type"]), tempfile.TemporaryDirectory() as directory:
                    path = Path(directory) / "data.sqlite3"
                    fixture = Path(directory) / "fixture.jsonl"
                    column = "geom_z" if dimension == 3 else "geom"
                    values = [self.feature(index, geometry) for index in (1, 2)]
                    fixture.write_text("".join(compact(f) + "\n" for f in values))
                    db = self.initialize(path)
                    db.execute("INSERT INTO gl_datasets VALUES('p','other',NULL)")
                    definition = compact({"supported": True, "fields": [], "reserved": [], "defaults": [], "dropped": [], "dynamic": True})
                    db.execute("INSERT INTO gl_schema_history VALUES('p','other',0,?)", (definition,))
                    db.execute("INSERT INTO gl_workspace_schemas VALUES('p','w','other',?)", (definition,))
                    db.commit()
                    sql = f"INSERT INTO gl_data_{'0' * 32}(feature_id,valid_from,properties,geometry_json,{column}) VALUES(?,1,?,?,coalesce(SetSRID(GeomFromGeoJSON(?),?),SetSRID(GeomFromGeoJSON(?),?)))"
                    g = compact(geometry)
                    with self.assertRaisesRegex(sqlite3.IntegrityError, "Geometry constraint"):
                        db.execute(sql, (values[0]["id"], compact(values[0]["properties"]), g, g, 4326, collection(geometry), 4326))
                    db.rollback()
                    db.execute(sql, (values[0]["id"], compact(values[0]["properties"]), g, g, 0, collection(geometry), 0))
                    db.commit(); db.close()
                    count = sqlite_seed(path, fixture, "p", "d", 1, dimension, os.environ["GL_SPATIALITE_EXTENSION"])
                    self.assertEqual(count, 2)
                    db = self.connect(path)
                    current = db.execute(f"SELECT feature_id,geometry_json,ST_SRID({column}) FROM gl_data_{'0' * 32} ORDER BY feature_id").fetchall()
                    self.assertEqual(current, [(f["id"], g, 0) for f in values])
                    self.assertEqual(db.execute("SELECT project,dataset,revision,definition FROM gl_schema_history").fetchall(), [("p", "other", 0, definition)])
                    self.assertEqual(db.execute("SELECT project,workspace,dataset,definition FROM gl_workspace_schemas").fetchall(), [("p", "w", "other", definition)])
                    db.close()

    def test_preflight_rejects_stale_format_and_geometry_contract(self):
        cases = [(STORAGE_FORMAT - 1, 0, None, "storage format"),
                 (STORAGE_FORMAT, 4326, None, "geometry contract")]
        for table in ["gl_schema_history", "gl_workspace_schemas"]:
            cases.extend([(STORAGE_FORMAT, 0, (table, "missing"), "schema contract"),
                          (STORAGE_FORMAT, 0, (table, "existing"), "existing schema state")])
        for version, srid, state, error in cases:
            with self.subTest(version=version, srid=srid, state=state), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "data.sqlite3"
                fixture = Path(directory) / "fixture.jsonl"
                fixture.write_text("")
                db = self.initialize(path, srid=srid, version=version)
                if state:
                    table, condition = state
                    if condition == "missing":
                        db.execute(f"DROP TABLE {table}")
                    elif table == "gl_schema_history":
                        db.execute("INSERT INTO gl_schema_history VALUES('p','d',0,'{}')")
                    else:
                        db.execute("INSERT INTO gl_workspace_schemas VALUES('p','w','d','{}')")
                    db.commit()
                db.close()
                with self.assertRaisesRegex(ValueError, error):
                    sqlite_seed(path, fixture, "p", "d", 1, 3, os.environ["GL_SPATIALITE_EXTENSION"])
                db = self.connect(path)
                self.assertEqual(db.execute("SELECT version FROM gl_format").fetchone()[0], version)
                self.assertEqual(db.execute(f"SELECT count(*) FROM gl_data_{'0' * 32}").fetchone()[0], 0)
                if state and state[1] == "existing":
                    self.assertEqual(db.execute(f"SELECT definition FROM {state[0]}").fetchall(), [("{}",)])
                db.close()

    def test_dimension_error_rolls_back_all_seed_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "data.sqlite3"
            fixture = Path(directory) / "fixture.jsonl"
            self.initialize(path).close()
            values = [self.feature(index, {"type": "Point", "coordinates": [1, 2, 3] if index < 3 else [1, 2]}) for index in (1, 2, 3)]
            fixture.write_text("".join(compact(f) + "\n" for f in values))
            with self.assertRaisesRegex(ValueError, "row 3 coordinate dimension"):
                sqlite_seed(path, fixture, "p", "d", 1, 3, os.environ["GL_SPATIALITE_EXTENSION"])
            db = self.connect(path)
            self.assertEqual(db.execute(f"SELECT count(*) FROM gl_data_{'0' * 32}").fetchone()[0], 0)
            db.close()


class SoakSummaryTests(unittest.TestCase):
    def test_tail_percentiles(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('soak', Path(__file__).with_name('load-incremental.py'))
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        self.assertEqual(module.percentile(list(range(1,101)), .95), 95)
        self.assertEqual(module.percentile([9], .99), 9)
        self.assertIsNone(module.percentile([], .95))

    def test_fast_replays_cannot_mask_slow_publications(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('soak', Path(__file__).with_name('load-incremental.py'))
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        summary, passed = module.statistics({'publish': [6000] * 10, 'replay': [1] * 1000}, 5000)
        self.assertFalse(passed)
        self.assertEqual(summary['publish']['count'], 10)
        self.assertEqual(summary['publish']['p95_ms'], 6000)
        self.assertEqual(summary['replay']['p95_ms'], 1)
        self.assertFalse(module.statistics({'publish': [1], 'replay': [6000]}, 5000)[1])


class CompletionTests(unittest.TestCase):
    def test_failed_load_keeps_integrity_evidence_without_passing(self):
        for failure in ({"soak": {"exit_code": 1}}, {"read_load_exit_code": 1}):
            result = {"success": False, "client": {"exit_code": 0},
                      "current_live": 10, "source_live": 10, **failure}
            with self.assertRaisesRegex(AssertionError, "integrity checked"):
                check_results(result, 10)
            self.assertTrue(result["integrity_verified"])
            self.assertFalse(result["success"])

    def test_corruption_and_client_failure_never_pass(self):
        result = {"success": False, "client": {"exit_code": 0},
                  "current_live": 10, "source_live": 9}
        with self.assertRaisesRegex(AssertionError, "background feature count"):
            check_results(result, 10)
        self.assertNotIn("integrity_verified", result)
        result["client"]["exit_code"] = 1
        check_results(result, 10)
        self.assertFalse(result["success"])
        result.update(client={"exit_code": 0}, source_live=10)
        check_results(result, 10)
        self.assertTrue(result["success"])


if __name__ == "__main__":
    unittest.main()
