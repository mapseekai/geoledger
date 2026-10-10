#!/usr/bin/env python3
import json
from pathlib import Path
import tempfile
import unittest

from benchmark_incremental import collection, rows, self_test_source_crud


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


if __name__ == "__main__":
    unittest.main()
