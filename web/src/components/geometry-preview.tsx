"use client";
import { previewShapes } from "@/lib/geometry-preview";
import { cn } from "@/lib/utils";
import { useMemo } from "react";

export type PreviewLayer = {
  id: string;
  label: string;
  geometry: unknown;
  /** CSS class suffix that picks the stroke colour. */
  tone: "base" | "current" | "draft" | "result" | "before" | "after";
  present: boolean;
};
const W = 280,
  H = 200;
/** Overlay of geometry versions on a blank, offline canvas. */
export function GeometryPreview({
  layers,
  className,
}: {
  layers: PreviewLayer[];
  className?: string;
}) {
  const shapes = useMemo(
    () =>
      previewShapes(
        layers.map((layer) => (layer.present ? layer.geometry : null)),
        { width: W, height: H, padding: 18 },
      ),
    [layers],
  );
  const empty = shapes.every((list) => list.length === 0);
  return (
    <figure className={cn("geo-preview", className)}>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        role="img"
        aria-label={`几何对比：${layers.map((l) => l.label).join("、")}`}
      >
        <defs>
          <pattern
            id="geo-grid"
            width="20"
            height="20"
            patternUnits="userSpaceOnUse"
          >
            <path d="M20 0H0V20" className="geo-grid-line" />
          </pattern>
        </defs>
        <rect width={W} height={H} fill="url(#geo-grid)" />
        {shapes.map((list, index) => (
          <g
            key={layers[index].id}
            className={`geo-layer is-${layers[index].tone}`}
          >
            {list.map((shape, i) =>
              shape.kind === "point" ? (
                <circle key={i} cx={shape.x} cy={shape.y} r={5} />
              ) : (
                <path
                  key={i}
                  d={shape.d}
                  className={shape.closed ? "is-closed" : undefined}
                />
              ),
            )}
          </g>
        ))}
        {empty && (
          <text x={W / 2} y={H / 2} textAnchor="middle" className="geo-empty">
            无几何
          </text>
        )}
      </svg>
      <figcaption>
        {layers.map((layer) => (
          <span
            key={layer.id}
            className={cn("geo-legend", `is-${layer.tone}`, {
              "is-missing": !layer.present,
            })}
          >
            <i aria-hidden="true" />
            {layer.label}
          </span>
        ))}
      </figcaption>
    </figure>
  );
}
