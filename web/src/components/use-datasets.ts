"use client";
import { call, type Dataset } from "@/lib/browser-api";
import { useEffect, useState } from "react";

/** Dataset id → name for one project, loaded once per mounted dialog. */
export function useDatasetNames(project: string) {
  const [names, setNames] = useState<Map<string, string>>(new Map());
  useEffect(() => {
    let active = true;
    (async () => {
      const result = new Map<string, string>();
      let after = "";
      for (let page = 0; page < 20; page++) {
        const rows = await call<Dataset[]>({
          action: "datasets",
          project,
          after: after || undefined,
          limit: 100,
        });
        for (const row of rows) result.set(row.id, row.name);
        if (rows.length < 100) break;
        after = rows.at(-1)!.id;
      }
      if (active) setNames(result);
    })().catch(() => {
      /* Names are cosmetic; identifiers remain visible. */
    });
    return () => {
      active = false;
    };
  }, [project]);
  return (id: string) => names.get(id) ?? id;
}
