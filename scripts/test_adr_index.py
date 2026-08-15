import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("adr-index.py")
SPEC = importlib.util.spec_from_file_location("adr_index", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class AdrStatusTests(unittest.TestCase):
    def test_frontmatter_status(self):
        self.assertEqual(MODULE.status_for("- Status: Accepted\n"), "Accepted")

    def test_heading_status(self):
        self.assertEqual(MODULE.status_for("## Status\n\nAccepted\n"), "Accepted")

    def test_missing_status(self):
        self.assertIsNone(MODULE.status_for("# ADR\n"))


if __name__ == "__main__":
    unittest.main()
