"""Filesystem policy regression tests; YAML examples cover syntax separately."""
import contextlib
import importlib.util
import io
import json
import subprocess
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("test_layout", Path(__file__).with_name("check-test-layout.py"))
layout = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(layout)


class TestLayout(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config = {
            "goRoots": ["sdk/go"], "tsRoots": {"web": "test"},
            "rustRoots": ["crates"],
            "generatedDirectories": ["sdk/go/internal/wire", "web/src/generated"],
            "integrationFiles": {},
        }
        for folder in ("sdk/go", "web", "crates"):
            (self.root / folder).mkdir(parents=True)

    def files(self, *names):
        for name in names:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// fixture\n")

    def errors(self):
        return layout.check_tree(self.root, self.config)[0]

    def test_go_unit_and_external_example_share_source_name(self):
        self.files("sdk/go/client.go", "sdk/go/client_test.go")
        (self.root / "sdk/go/client_test.go").write_text("package geoledger_test\nfunc ExampleDial() {}")
        self.assertEqual(self.errors(), [])

    def test_go_split_or_orphan_rejected(self):
        for name in ("client_internal_test.go", "example_test.go", "missing_test.go"):
            with self.subTest(name=name):
                self.files("sdk/go/client.go", "sdk/go/" + name)
                self.assertTrue(self.errors())
                (self.root / "sdk/go" / name).unlink()

    def test_legitimate_go_source_suffix_is_not_mechanically_rejected(self):
        self.files("sdk/go/client_linux.go", "sdk/go/client_linux_test.go")
        self.assertEqual(self.errors(), [])

    def test_ts_and_tsx_require_exact_extension_and_neighbor(self):
        self.files("web/src/a.ts", "web/src/a.test.ts", "web/src/b.tsx", "web/src/b.test.tsx")
        self.assertEqual(self.errors(), [])
        self.files("web/src/b.test.ts")
        self.assertTrue(self.errors())

    def test_ts_split_file_cannot_claim_an_existing_owner(self):
        self.files("web/src/a.ts", "web/src/a.test.ts", "web/src/a.extra.test.ts")
        self.assertTrue(self.errors())

    def test_consistent_spec_is_supported(self):
        self.config["tsRoots"]["web"] = "spec"
        self.files("web/src/a.ts", "web/src/a.spec.ts")
        self.assertEqual(self.errors(), [])
        self.files("web/src/a.test.ts")
        self.assertTrue(any("multiple" in error for error in self.errors()))
        self.assertTrue(any("consistently" in error for error in self.errors()))

    def test_uncompiled_cjs_unit_is_not_a_ts_filename_exception(self):
        self.files("web/src/a.ts", "web/src/a.test.cjs")
        self.assertTrue(self.errors())

    def test_test_directory_requires_explicit_integration_reason(self):
        name = "web/test/http.test.ts"
        self.files(name)
        self.assertTrue(self.errors())
        self.config["integrationFiles"][name] = "HTTP boundary + operation dispatch"
        self.assertEqual(self.errors(), [])

    def test_blank_or_stale_integration_registration_fails(self):
        name = "web/test/http.test.ts"
        self.config["integrationFiles"][name] = ""
        self.assertTrue(self.errors())
        self.files(name)
        self.assertTrue(self.errors())

    def test_integration_cannot_be_registered_beside_production(self):
        name = "web/src/http.test.ts"
        self.files(name)
        self.config["integrationFiles"][name] = "claimed integration"
        self.assertTrue(self.errors())

    def test_go_real_integration_can_use_test_directory(self):
        name = "sdk/go/test/transport_test.go"
        self.files(name)
        self.config["integrationFiles"][name] = "real loopback service boundary"
        self.assertEqual(self.errors(), [])

    def test_ts_go_plural_tests_directory_is_rejected(self):
        self.files("web/tests/a.test.ts", "sdk/go/tests/a_test.go")
        self.assertEqual(len(self.errors()), 2)

    def test_rust_unit_files_are_rejected_and_cargo_integration_is_allowed(self):
        self.files("crates/core/Cargo.toml", "crates/core/src/lib.rs", "crates/core/tests/public_api.rs")
        self.assertEqual(self.errors(), [])
        self.files("crates/core/src/review_tests.rs", "crates/core/src/tests.rs")
        self.assertEqual(len(self.errors()), 2)

    def test_rust_nested_tests_directory_does_not_impersonate_cargo(self):
        self.files("crates/core/Cargo.toml", "crates/core/src/tests/check.rs")
        self.assertTrue(self.errors())

    def test_paths_in_strings_and_comments_do_not_create_files(self):
        self.files("web/src/a.ts", "crates/core/Cargo.toml", "crates/core/src/contest.rs")
        (self.root / "web/src/a.ts").write_text('// src/a.extra.test.ts\nconst path = "tests/orphan_test.go";')
        self.assertEqual(self.errors(), [])

    def test_generated_and_build_artifacts_are_explicitly_excluded(self):
        self.files("sdk/go/internal/wire/generated_test.go", "web/src/generated/missing.test.ts", "web/node_modules/bad.test.ts", "crates/core/target/old_tests.rs")
        self.assertEqual(self.errors(), [])

    def test_missing_language_root_is_configuration_failure(self):
        self.config["goRoots"] = ["missing"]
        with self.assertRaises(ValueError):
            self.errors()

    def test_missing_ast_dependency_is_nonzero_and_actionable(self):
        with patch.object(layout.shutil, "which", return_value=None), contextlib.redirect_stderr(io.StringIO()) as output:
            self.assertEqual(layout.main(), 2)
        self.assertIn("@ast-grep/cli@0.51.0", output.getvalue())

    def test_ast_configuration_error_cannot_become_a_clean_scan(self):
        result = subprocess.CompletedProcess([], 2, "[]", "invalid rule")
        with patch.object(layout.subprocess, "run", return_value=result), self.assertRaises(ValueError):
            layout.run_json(["ast-grep"], self.root)

    def test_invalid_ast_json_cannot_become_a_clean_scan(self):
        result = subprocess.CompletedProcess([], 0, "not JSON", "")
        with patch.object(layout.subprocess, "run", return_value=result), self.assertRaises(ValueError):
            layout.run_json(["ast-grep"], self.root)

    def test_rust_duplicate_containers_use_ast_records(self):
        self.files("crates/core/Cargo.toml", "crates/core/src/lib.rs")
        version = subprocess.CompletedProcess([], 0, "ast-grep 0.51.0\n", "")
        nodes = [{"file": "crates/core/src/lib.rs", "range": {"start": {"line": line}}} for line in (1, 5)]
        self.install_ast_rules()
        with patch.object(layout.subprocess, "run", return_value=version), patch.object(layout, "run_json", side_effect=[[]] * len(layout.AST_RULES) + [nodes]):
            self.assertTrue(any("multiple inline" in e for e in layout.check_ast(self.root, "ast-grep", self.config)))

    def test_no_rust_unit_container_is_allowed(self):
        result = subprocess.CompletedProcess([], 1, "[]", "")
        with patch.object(layout.subprocess, "run", return_value=result):
            self.assertEqual(layout.run_json(["ast-grep"], self.root, allow_no_match=True), [])
            with self.assertRaises(ValueError):
                layout.run_json(["ast-grep"], self.root)

    def install_ast_rules(self):
        source = Path(__file__).resolve().parent.parent / ".ast-grep/rules"
        shutil.copytree(source, self.root / ".ast-grep/rules")

    def test_ast_respects_real_go_example_and_generated_file_paths(self):
        self.install_ast_rules()
        self.files("sdk/go/client_test.go", "sdk/go/internal/geoledgerv1/generated.go")
        for name in ("sdk/go/client_test.go", "sdk/go/internal/geoledgerv1/generated.go"):
            (self.root / name).write_text("package p\nfunc ExampleDial() {}\n")
        self.assertEqual(layout.check_ast(self.root, shutil.which("ast-grep"), self.config), [])
        self.files("sdk/go/client.go")
        (self.root / "sdk/go/client.go").write_text("package p\nfunc TestRead(t *testing.T) {}\n")
        self.assertTrue(layout.check_ast(self.root, shutil.which("ast-grep"), self.config))

    def test_ast_ts_path_and_comment_boundaries(self):
        self.install_ast_rules()
        self.files("web/src/a.ts", "web/src/a.test.ts", "web/src/consistent.spec.ts", "web/test/integration.test.ts")
        (self.root / "web/src/a.ts").write_text('// import test from "node:test";\nconst path = "test/a.test.ts";')
        for name in ("web/src/a.test.ts", "web/src/consistent.spec.ts", "web/test/integration.test.ts"):
            (self.root / name).write_text('import test from "node:test";')
        self.assertEqual(layout.check_ast(self.root, shutil.which("ast-grep"), self.config), [])
        (self.root / "web/src/a.ts").write_text('import test from "node:test";')
        self.assertTrue(layout.check_ast(self.root, shutil.which("ast-grep"), self.config))

    def test_missing_required_rule_is_nonzero_instead_of_empty_filter(self):
        version = subprocess.CompletedProcess([], 0, "ast-grep 0.51.0\n", "")
        with patch.object(layout.subprocess, "run", return_value=version), self.assertRaises(ValueError):
            layout.check_ast(self.root, "ast-grep", self.config)


if __name__ == "__main__":
    unittest.main()
