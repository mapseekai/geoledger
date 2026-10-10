#!/usr/bin/env python3
"""Measure create-time file upload and first map display without loading the file in Python.

Run on a disposable Console. Full source/readback integrity is covered by the Rust benchmark.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time
import uuid
from playwright.sync_api import sync_playwright, expect

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("--url", required=True)
p.add_argument("--token-file", required=True)
p.add_argument("--chromium", required=True)
p.add_argument("--geojson", required=True)
p.add_argument("--expected-features", required=True, type=int)
p.add_argument("--report", required=True)
p.add_argument("--timeout", type=int, default=1200)
p.add_argument("--browser-rss-limit-mib", type=int, default=4096)
a = p.parse_args()
report = {"file": a.geojson, "bytes": Path(a.geojson).stat().st_size, "expected_features": a.expected_features, "save_requests": 0, "saved_features": 0, "page_errors": [], "browser_rss_peak_mib": 0, "success": False}
started = time.monotonic()
with sync_playwright() as playwright:
    browser = playwright.chromium.launch(headless=True, executable_path=a.chromium)
    page = browser.new_page(viewport={"width": 1440, "height": 1000})
    page.set_default_timeout(30000)
    page.on("pageerror", lambda error: report["page_errors"].append(str(error)))
    session = browser.new_browser_cdp_session()
    def sample_memory():
        processes = session.send("SystemInfo.getProcessInfo")["processInfo"]
        pids = [str(int(process["id"])) for process in processes]
        raw = subprocess.run(["ps", "-o", "rss=", "-p", ",".join(pids)], capture_output=True, text=True).stdout
        rss = sum(int(line) for line in raw.splitlines() if line.strip()) / 1024
        report["browser_rss_peak_mib"] = max(report["browser_rss_peak_mib"], rss)
        if rss > a.browser_rss_limit_mib:
            raise RuntimeError(f"isolated browser exceeded {a.browser_rss_limit_mib} MiB RSS safety ceiling")
    def response(request):
        if "/api/console" not in request.url or request.method != "POST":
            return
        body = request.post_data_json or {}
        if body.get("action") == "save":
            report["save_requests"] += 1
            result = request.response()
            if result and result.status == 200:
                report["saved_features"] += len(body.get("edits", []))
                if report["save_requests"] % 500 == 0:
                    print(json.dumps({"phase": "upload", "saved_features": report["saved_features"], "seconds": time.monotonic() - started}), flush=True)
            elif result:
                report["save_error"] = {"status": result.status, "body": result.text()[:500]}
    page.on("requestfinished", response)
    def call(body):
        result = page.evaluate("""async body => { const response = await fetch('/api/console', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});return {status:response.status,body:await response.text()}; }""", body)
        if result["status"] != 200:
            raise RuntimeError(str(result))
        return json.loads(result["body"])
    try:
        token = json.loads(Path(a.token_file).read_text())[0]["token"]
        page.goto(a.url + "/login")
        page.get_by_label("访问令牌", exact=True).fill(token)
        page.get_by_role("button", name="进入控制台").click()
        page.wait_for_url("**/projects")
        project = call({"action": "createProject", "name": "file-benchmark-" + uuid.uuid4().hex[:8]})["id"]
        page.goto(a.url + "/datasets?project=" + project)
        page.get_by_role("button", name="创建数据集", exact=True).click()
        page.get_by_label("名称", exact=True).fill("Real file")
        page.get_by_role("combobox", name="数据来源", exact=True).click()
        page.get_by_role("option", name="上传 GeoJSON 文件", exact=True).click()
        page.get_by_label("GeoJSON 文件", exact=True).set_input_files(a.geojson, timeout=240000)
        phase = time.monotonic()
        page.get_by_role("button", name="创建", exact=True).click()
        while page.get_by_role("dialog").count():
            sample_memory()
            for alert in page.get_by_role("dialog").get_by_role("alert").all():
                if alert.inner_text().strip():
                    raise RuntimeError(alert.inner_text())
            if time.monotonic() - phase > a.timeout:
                raise RuntimeError("upload deadline exceeded")
            page.wait_for_timeout(1000)
        report["upload_seconds"] = time.monotonic() - phase
        assert report["saved_features"] == a.expected_features, "saved count differs from complete file"
        phase = time.monotonic()
        page.locator(".map-progress").wait_for(state="hidden", timeout=120000)
        expect(page.locator(".map-canvas canvas")).to_be_visible()
        report["first_map_seconds"] = time.monotonic() - phase
        report["display_footer"] = page.locator(".gis-panel-foot").first.inner_text()
        report["project"] = project
        sample_memory()
        assert not report["page_errors"], report["page_errors"]
        report["success"] = True
    except Exception as error:
        report["error"] = str(error)
    finally:
        report["seconds"] = time.monotonic() - started
        try:
            report["screen_text"] = page.locator("body").inner_text()[-3000:]
            page.screenshot(path=a.report + ".png")
        except Exception:
            pass
        browser.close()
        Path(a.report).write_text(json.dumps(report, ensure_ascii=False, indent=2))
        print(json.dumps(report, ensure_ascii=False), flush=True)
if not report["success"]:
    raise SystemExit(1)
