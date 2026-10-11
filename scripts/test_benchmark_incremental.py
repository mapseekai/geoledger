#!/usr/bin/env python3
import json
from pathlib import Path
import tempfile
import unittest

from benchmark_incremental import collection, rows, self_test_source_crud, check_results


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
                      "history_live": 10, "source_live": 10, **failure}
            with self.assertRaisesRegex(AssertionError, "integrity checked"):
                check_results(result, 10)
            self.assertTrue(result["integrity_verified"])
            self.assertFalse(result["success"])

    def test_corruption_and_client_failure_never_pass(self):
        result = {"success": False, "client": {"exit_code": 0},
                  "history_live": 10, "source_live": 9}
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
