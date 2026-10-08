"use client";
import type {
  ExpressionSpecification,
  FilterSpecification,
  GeoJSONSource,
  LayerSpecification,
  Map as LibreMap,
  StyleSpecification,
} from "maplibre-gl";
import type { FeatureCollection } from "geojson";
import { Expand, Layers, Minus, Plus, Scan, Shrink } from "lucide-react";
import {
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type ReactNode,
  type Ref,
} from "react";
import {
  contains,
  formatLngLat,
  mercatorBounds,
  type Bounds,
  type DrawKind,
} from "@/lib/map";
import { useRasterBasemap } from "./map-config";

export type MapHandle = {
  fit: (bounds: Bounds, maxZoom?: number) => void;
  reveal: (bounds: Bounds) => void;
};
export type BasemapId = "light" | "dark" | "raster";
export const kindColors: Record<DrawKind, string> = {
  point: "#f97316",
  line: "#2563eb",
  polygon: "#16a34a",
};
const SELECT = "#facc15";
const backgrounds: Record<BasemapId, string> = {
  light: "#f2f4f7",
  dark: "#1b2230",
  raster: "#f2f4f7",
};
const selected: ExpressionSpecification = [
  "boolean",
  ["feature-state", "selected"],
  false,
];
const hovered: ExpressionSpecification = [
  "boolean",
  ["feature-state", "hover"],
  false,
];
const only = (type: string): FilterSpecification => ["==", "$type", type];
const round = { "line-cap": "round", "line-join": "round" } as const;
const dataLayers: LayerSpecification[] = [
  {
    id: "polygon-fill",
    type: "fill",
    source: "features",
    filter: only("Polygon"),
    paint: {
      "fill-color": ["case", selected, SELECT, kindColors.polygon],
      "fill-opacity": ["case", selected, 0.45, hovered, 0.32, 0.18],
    },
  },
  {
    id: "polygon-outline",
    type: "line",
    source: "features",
    filter: only("Polygon"),
    layout: round,
    paint: {
      "line-color": ["case", selected, "#a16207", "#15803d"],
      "line-width": ["case", selected, 3, hovered, 2.5, 1.5],
    },
  },
  {
    id: "line-halo",
    type: "line",
    source: "features",
    filter: only("LineString"),
    layout: round,
    paint: {
      "line-color": SELECT,
      "line-width": ["case", selected, 10, hovered, 8, 0],
      "line-opacity": ["case", selected, 0.95, hovered, 0.4, 0],
    },
  },
  {
    id: "line",
    type: "line",
    source: "features",
    filter: only("LineString"),
    layout: round,
    paint: {
      "line-color": ["case", selected, "#1e3a8a", kindColors.line],
      "line-width": ["case", selected, 4, hovered, 3.5, 2.5],
    },
  },
  {
    id: "point",
    type: "circle",
    source: "features",
    filter: only("Point"),
    paint: {
      "circle-color": ["case", selected, SELECT, kindColors.point],
      "circle-radius": [
        "interpolate",
        ["linear"],
        ["zoom"],
        9,
        ["case", selected, 6.5, hovered, 5.5, 3.5],
        14,
        ["case", selected, 8, hovered, 7.5, 6],
      ],
      "circle-stroke-color": ["case", selected, "#111827", "#ffffff"],
      "circle-stroke-width": ["case", selected, 2.5, 1.5],
    },
  },
];
const kindLayers: Record<DrawKind, string[]> = {
  point: ["point"],
  line: ["line-halo", "line"],
  polygon: ["polygon-fill", "polygon-outline"],
};
const interactive = ["point", "line", "polygon-outline", "polygon-fill"];
const empty: FeatureCollection = { type: "FeatureCollection", features: [] };

function storedBasemap(raster: boolean): BasemapId {
  if (typeof window === "undefined") return "light";
  const value = window.sessionStorage.getItem("gl.basemap");
  return value === "dark" || (value === "raster" && raster) ? value : "light";
}

export function MapView({
  ref,
  collection,
  bounds,
  fitKey,
  selected: selectedId,
  hovered: hoveredId,
  visible,
  onSelect,
  onHover,
  describe,
  fullscreen,
  toggleFullscreen,
  children,
}: {
  ref?: Ref<MapHandle>;
  collection: FeatureCollection;
  bounds: Bounds | null;
  /** Fits the data extent whenever this key changes and bounds exist. */
  fitKey: string;
  selected?: number;
  hovered?: number;
  visible: Record<DrawKind, boolean>;
  onSelect: (index?: number) => void;
  onHover: (index?: number) => void;
  describe: (index: number) => string;
  fullscreen: boolean;
  toggleFullscreen: () => void;
  children?: ReactNode;
}) {
  const raster = useRasterBasemap();
  const container = useRef<HTMLDivElement>(null);
  const mapRef = useRef<LibreMap | undefined>(undefined);
  const readout = useRef<HTMLSpanElement>(null);
  const tip = useRef<HTMLDivElement>(null);
  const callbacks = useRef({ onSelect, onHover, describe });
  callbacks.current = { onSelect, onHover, describe };
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");
  const [zoom, setZoom] = useState<number>();
  const [basemap, setBasemap] = useState<BasemapId>("light");
  const [menu, setMenu] = useState(false);
  const [tileError, setTileError] = useState(false);
  const fitted = useRef("");
  const state = useRef<{ selected?: number; hover?: number }>({});

  useEffect(() => setBasemap(storedBasemap(!!raster)), [raster]);

  useEffect(() => {
    let cancelled = false;
    let observer: ResizeObserver | undefined;
    async function init() {
      try {
        const lib = await import("maplibre-gl");
        if (cancelled || !container.current) return;
        lib.setWorkerUrl(
          new URL("maplibre-gl/dist/maplibre-gl-worker.mjs", import.meta.url)
            .href,
        );
        const map = new lib.Map({
          container: container.current,
          attributionControl: false,
          dragRotate: false,
          pitchWithRotate: false,
          maxPitch: 0,
          center: [105, 35],
          zoom: 3,
          style: {
            version: 8,
            sources: { features: { type: "geojson", data: empty } },
            layers: [
              {
                id: "background",
                type: "background",
                paint: { "background-color": backgrounds.light },
              },
              ...dataLayers,
            ],
          } satisfies StyleSpecification,
        });
        mapRef.current = map;
        map.touchZoomRotate.disableRotation();
        map.keyboard.disableRotation();
        map.addControl(
          new lib.ScaleControl({ maxWidth: 110, unit: "metric" }),
          "bottom-left",
        );
        const pick = (x: number, y: number) =>
          map.queryRenderedFeatures(
            [
              [x - 5, y - 5],
              [x + 5, y + 5],
            ],
            { layers: interactive.filter((id) => map.getLayer(id)) },
          )[0]?.id as number | undefined;
        map.on("mousemove", (e) => {
          if (readout.current)
            readout.current.textContent = formatLngLat(
              e.lngLat.lng,
              e.lngLat.lat,
            );
          const id = pick(e.point.x, e.point.y);
          map.getCanvas().style.cursor = id === undefined ? "" : "pointer";
          callbacks.current.onHover(id);
          if (tip.current) {
            tip.current.hidden = id === undefined;
            if (id !== undefined) {
              tip.current.textContent = callbacks.current.describe(id);
              tip.current.style.transform = `translate(${e.point.x + 14}px, ${e.point.y + 14}px)`;
            }
          }
        });
        map.getCanvasContainer().addEventListener("mouseleave", () => {
          if (readout.current) readout.current.textContent = "";
          if (tip.current) tip.current.hidden = true;
          callbacks.current.onHover(undefined);
        });
        map.on("click", (e) =>
          callbacks.current.onSelect(pick(e.point.x, e.point.y)),
        );
        map.on("zoom", () => setZoom(map.getZoom()));
        map.on("error", (event) => {
          if (cancelled) return;
          if ((event as { sourceId?: string }).sourceId === "basemap")
            setTileError(true);
          else setError(event.error?.message ?? "地图渲染失败");
        });
        map.once("load", () => {
          if (cancelled) return;
          setZoom(map.getZoom());
          setReady(true);
        });
        observer = new ResizeObserver(() => map.resize());
        observer.observe(container.current);
      } catch (e) {
        if (!cancelled)
          setError(e instanceof Error ? e.message : "地图无法初始化");
      }
    }
    void init();
    return () => {
      cancelled = true;
      observer?.disconnect();
      mapRef.current?.remove();
      mapRef.current = undefined;
    };
  }, []);

  const map = ready ? mapRef.current : undefined;
  const fit = (b: Bounds, maxZoom = 16) => {
    const m = mapRef.current;
    if (!m) return;
    const small = m.getContainer().clientWidth < 520;
    m.fitBounds(mercatorBounds(b), {
      padding: small ? 28 : 56,
      maxZoom,
      duration: 350,
    });
  };
  useImperativeHandle(ref, () => ({
    fit,
    reveal: (b) => {
      const m = mapRef.current;
      if (!m) return;
      const view = m.getBounds();
      if (
        !contains(
          [view.getWest(), view.getSouth(), view.getEast(), view.getNorth()],
          mercatorBounds(b),
        )
      )
        fit(b, Math.max(m.getZoom(), 14));
    },
  }));

  useEffect(() => {
    (map?.getSource("features") as GeoJSONSource | undefined)?.setData(
      collection,
    );
  }, [map, collection]);

  useEffect(() => {
    if (!map || !bounds || fitted.current === fitKey) return;
    fitted.current = fitKey;
    map.fitBounds(mercatorBounds(bounds), {
      padding: map.getContainer().clientWidth < 520 ? 28 : 56,
      maxZoom: 16,
      duration: 0,
    });
  }, [map, bounds, fitKey]);

  useEffect(() => {
    if (!map) return;
    const apply = (key: "selected" | "hover", next?: number) => {
      const prev = state.current[key];
      if (prev !== undefined)
        map.setFeatureState({ source: "features", id: prev }, { [key]: false });
      if (next !== undefined)
        map.setFeatureState({ source: "features", id: next }, { [key]: true });
      state.current[key] = next;
    };
    apply("selected", selectedId);
    apply("hover", hoveredId);
  }, [map, collection, selectedId, hoveredId]);

  useEffect(() => {
    if (!map) return;
    for (const kind of Object.keys(kindLayers) as DrawKind[])
      for (const id of kindLayers[kind])
        map.setLayoutProperty(
          id,
          "visibility",
          visible[kind] ? "visible" : "none",
        );
  }, [map, visible]);

  useEffect(() => {
    if (!map) return;
    map.setPaintProperty(
      "background",
      "background-color",
      backgrounds[basemap],
    );
    // Lighter strokes keep lines and outlines legible on the dark basemap.
    const dark = basemap === "dark";
    map.setPaintProperty("line", "line-color", [
      "case",
      selected,
      dark ? "#fef08a" : "#1e3a8a",
      dark ? "#60a5fa" : kindColors.line,
    ]);
    map.setPaintProperty("polygon-outline", "line-color", [
      "case",
      selected,
      dark ? "#fef08a" : "#a16207",
      dark ? "#4ade80" : "#15803d",
    ]);
    const has = !!map.getSource("basemap");
    if (basemap === "raster" && raster && !has) {
      setTileError(false);
      map.addSource("basemap", {
        type: "raster",
        tiles: [raster.url],
        tileSize: 256,
        maxzoom: 19,
        attribution: raster.attribution,
      });
      map.addLayer(
        { id: "basemap", type: "raster", source: "basemap" },
        "polygon-fill",
      );
    } else if (basemap !== "raster" && has) {
      map.removeLayer("basemap");
      map.removeSource("basemap");
    }
  }, [map, basemap, raster]);

  const choose = (id: BasemapId) => {
    setBasemap(id);
    setMenu(false);
    window.sessionStorage.setItem("gl.basemap", id);
  };
  const options: [BasemapId, string][] = [
    ["light", "浅色"],
    ["dark", "深色"],
    ...(raster ? ([["raster", "在线地图"]] as [BasemapId, string][]) : []),
  ];
  return (
    <div className={`map-view is-${basemap}`}>
      <div
        ref={container}
        className="map-canvas"
        aria-label="地图"
        role="region"
      />
      <div ref={tip} className="map-tip" hidden />
      {children}
      {error && (
        <div className="map-overlay" role="alert">
          地图无法加载：{error}
        </div>
      )}
      <div className="map-controls" role="toolbar" aria-label="地图工具">
        <div className="map-control-group">
          <button
            type="button"
            title="放大"
            aria-label="放大"
            disabled={!map}
            onClick={() => map?.zoomIn()}
          >
            <Plus />
          </button>
          <button
            type="button"
            title="缩小"
            aria-label="缩小"
            disabled={!map}
            onClick={() => map?.zoomOut()}
          >
            <Minus />
          </button>
        </div>
        <div className="map-control-group">
          <button
            type="button"
            title="缩放至图层"
            aria-label="缩放至图层"
            disabled={!map || !bounds}
            onClick={() => bounds && fit(bounds)}
          >
            <Scan />
          </button>
          <button
            type="button"
            title={fullscreen ? "退出全屏" : "全屏"}
            aria-label={fullscreen ? "退出全屏" : "全屏"}
            onClick={toggleFullscreen}
          >
            {fullscreen ? <Shrink /> : <Expand />}
          </button>
        </div>
        <div className="map-control-group map-basemap">
          <button
            type="button"
            title="底图"
            aria-label="底图"
            aria-expanded={menu}
            onClick={() => setMenu((v) => !v)}
          >
            <Layers />
          </button>
          {menu && (
            <div
              className="map-basemap-menu"
              role="radiogroup"
              aria-label="底图"
            >
              {options.map(([id, label]) => (
                <button
                  type="button"
                  key={id}
                  role="radio"
                  aria-checked={basemap === id}
                  onClick={() => choose(id)}
                >
                  <span
                    className={`basemap-swatch is-${id}`}
                    aria-hidden="true"
                  />
                  {label}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
      <div className="map-status">
        <span ref={readout} className="mono" aria-label="光标经纬度" />
        {zoom !== undefined && (
          <span className="mono">Z {zoom.toFixed(1)}</span>
        )}
        <span>WGS 84</span>
        {basemap === "raster" && raster && (
          <span className={tileError ? "is-error" : undefined}>
            {tileError ? "底图加载失败" : raster.attribution}
          </span>
        )}
      </div>
    </div>
  );
}
