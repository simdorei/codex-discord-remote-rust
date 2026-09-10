from __future__ import annotations

import unittest

import codex_system_resources as resources


class SystemResourcesTests(unittest.TestCase):
    def test_formats_resource_snapshot_with_free_space_first(self) -> None:
        snapshot = resources.SystemResourceSnapshot(
            cpu_percent=12.5,
            memory_total_bytes=16 * 1024**3,
            memory_free_bytes=4 * 1024**3,
            disk_label="C:",
            disk_total_bytes=512 * 1024**3,
            disk_free_bytes=128 * 1024**3,
        )

        message = resources.format_system_resources(snapshot)

        self.assertEqual(
            message,
            "\n".join(
                [
                    "System resources",
                    "cpu: 12.5%",
                    "ram_free: 4.0 GiB / 16.0 GiB (25.0% free)",
                    "disk_free C: 128.0 GiB / 512.0 GiB (25.0% free)",
                ]
            ),
        )

    def test_parses_powershell_metric_lines(self) -> None:
        snapshot = resources.parse_powershell_resource_metrics(
            "\n".join(
                [
                    "cpu_percent=45",
                    "memory_total_kib=8388608",
                    "memory_free_kib=2097152",
                    "disk_label=D:",
                    "disk_total_bytes=1073741824",
                    "disk_free_bytes=268435456",
                ]
            )
        )

        self.assertEqual(snapshot.cpu_percent, 45.0)
        self.assertEqual(snapshot.memory_total_bytes, 8 * 1024**3)
        self.assertEqual(snapshot.memory_free_bytes, 2 * 1024**3)
        self.assertEqual(snapshot.disk_label, "D:")
        self.assertEqual(snapshot.disk_total_bytes, 1024**3)
        self.assertEqual(snapshot.disk_free_bytes, 256 * 1024**2)


if __name__ == "__main__":
    _ = unittest.main()
