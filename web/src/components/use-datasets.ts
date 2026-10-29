"use client";
import { loadDatasetNames } from "@/lib/dataset-names";
import { useEffect, useState } from "react";

/** Dataset id → name for one project, loaded once per mounted dialog. */
export function useDatasetNames(project: string) {
  const [names, setNames] = useState<Map<string, string>>(new Map());
  useEffect(() => {
    const controller = new AbortController();
    loadDatasetNames(project, controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) setNames(result);
      })
      .catch(() => {
        /* Names are cosmetic; cancellation and failures leave identifiers visible. */
      });
    return () => controller.abort();
  }, [project]);
  return (id: string) => names.get(id) ?? id;
}
