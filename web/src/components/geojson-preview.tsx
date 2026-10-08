"use client";

import { useEffect, useRef, useState } from "react";
import type { Map as LibreMap } from "maplibre-gl";
import { previewGeometry } from "@/lib/geojson";

export function GeoJSONPreview({ raw }: { raw: string }) {
  const container = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState("正在加载地图…");
  useEffect(() => {
    let cancelled = false;
    let map: LibreMap | undefined;
    let observer: ResizeObserver | undefined;
    setStatus("正在加载地图…");
    async function render() {
      try {
        const { data, bounds } = previewGeometry(raw);
        if (!bounds) {
          setStatus(
            "该要素的属性和 GeoJSON 原文如下。添加几何坐标后可在此预览地图。",
          );
          return;
        }
        const { Map, NavigationControl, ScaleControl, setWorkerUrl } =
          await import("maplibre-gl");
        if (cancelled || !container.current) return;
        setWorkerUrl(
          new URL("maplibre-gl/dist/maplibre-gl-worker.mjs", import.meta.url)
            .href,
        );
        map = new Map({
          container: container.current,
          attributionControl: false,
          style: {
            version: 8,
            sources: { feature: { type: "geojson", data } },
            layers: [
              {
                id: "background",
                type: "background",
                paint: { "background-color": "#eef2f6" },
              },
              {
                id: "area",
                type: "fill",
                source: "feature",
                filter: ["==", "$type", "Polygon"],
                paint: { "fill-color": "#ea580c", "fill-opacity": 0.25 },
              },
              {
                id: "line",
                type: "line",
                source: "feature",
                filter: ["!=", "$type", "Point"],
                paint: { "line-color": "#c2410c", "line-width": 3 },
              },
              {
                id: "point",
                type: "circle",
                source: "feature",
                filter: ["==", "$type", "Point"],
                paint: {
                  "circle-color": "#ea580c",
                  "circle-radius": 7,
                  "circle-stroke-color": "#ffffff",
                  "circle-stroke-width": 2,
                },
              },
            ],
          },
          locale: {
            "NavigationControl.ZoomIn": "放大",
            "NavigationControl.ZoomOut": "缩小",
            "NavigationControl.ResetBearing": "朝向正北",
          },
        });
        map.addControl(
          new NavigationControl({ showCompass: false }),
          "top-right",
        );
        map.addControl(new ScaleControl());
        // Web Mercator's finite latitude range also allows previews at the poles.
        const latitude = (y: number) =>
          Math.max(-85.051129, Math.min(85.051129, y));
        map.fitBounds(
          [bounds[0], latitude(bounds[1]), bounds[2], latitude(bounds[3])],
          { padding: 40, maxZoom: 15, duration: 0 },
        );
        map.on("error", (event) => {
          if (!cancelled)
            setStatus(
              `地图加载失败：${event.error.message}。可查看下方 GeoJSON 原文。`,
            );
        });
        map.once("idle", () => {
          if (!cancelled) setStatus("地图已就绪");
        });
        observer = new ResizeObserver(() => map?.resize());
        observer.observe(container.current);
      } catch (error) {
        if (!cancelled)
          setStatus(
            error instanceof Error
              ? error.message
              : "地图加载失败，可查看下方 GeoJSON 原文。",
          );
      }
    }
    void render();
    return () => {
      cancelled = true;
      observer?.disconnect();
      map?.remove();
    };
  }, [raw]);
  return (
    <section className="geojson-preview" aria-label="GeoJSON 地图预览">
      <div
        ref={container}
        className="geojson-map"
        hidden={status !== "地图已就绪" && status !== "正在加载地图…"}
      />
      <p role="status">{status} · 几何预览（WGS 84 经纬度）</p>
    </section>
  );
}
