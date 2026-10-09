export type GraphEntry = { id: string; base?: string; draft?: boolean };

export type GraphNode = {
  id: string;
  row: number;
  lane: number;
  draft: boolean;
};

export type GraphEdge = {
  from: string;
  fromRow: number;
  fromLane: number;
  to?: string;
  toRow?: number;
  toLane?: number;
  boundary?: "top" | "bottom";
  draft: boolean;
};

/** Lays out source-base parent DAGs, including merge parents for skipped releases. */
export function layoutGraph(entries: GraphEntry[]) {
  const rows = new Map(entries.map((entry, row) => [entry.id, row]));
  const revisions = entries.flatMap((entry) => {
    const match = /^r(\d+)$/.exec(entry.id);
    return match ? [BigInt(match[1])] : [];
  });
  const oldest = revisions.reduce<bigint | undefined>(
    (value, revision) =>
      value === undefined || revision < value ? revision : value,
    undefined,
  );
  const newest = revisions.reduce<bigint | undefined>(
    (value, revision) =>
      value === undefined || revision > value ? revision : value,
    undefined,
  );
  const nodes = entries.map((entry, row) => ({
    id: entry.id,
    row,
    lane: -1,
    draft: Boolean(entry.draft),
  }));
  const edges: GraphEdge[] = [];
  const arrivals = entries.map(
    () => [] as { lane: number; primary: boolean }[],
  );
  const published = entries
    .map((entry, row) => ({ entry, row }))
    .filter(({ entry }) => !entry.draft && /^r\d+$/.test(entry.id));

  for (let index = published.length - 1; index >= 0; index -= 1) {
    const { entry, row } = published[index];
    const incoming = arrivals[row].sort(
      (left, right) =>
        Number(right.primary) - Number(left.primary) || left.lane - right.lane,
    );
    const lane = incoming[0]?.lane ?? 0;
    arrivals[row] = [];
    nodes[row].lane = lane;
    const revision = BigInt(entry.id.slice(1));
    if (entry.base === undefined || !/^\d+$/.test(entry.base)) continue;
    const base = BigInt(entry.base);
    const parentIds: { id: string; primary: boolean }[] = [];
    if (base !== 0n) parentIds.push({ id: `r${base}`, primary: true });
    if (base !== revision - 1n) {
      parentIds.push({ id: `r${revision - 1n}`, primary: false });
    }

    for (const parent of parentIds) {
      const parentRow = rows.get(parent.id);
      if (parentRow !== undefined) {
        if (parent.primary) {
          arrivals[parentRow].push({ lane, primary: true });
        } else {
          const occupied = new Set([
            lane,
            ...arrivals.flatMap((items) => items.map((item) => item.lane)),
          ]);
          let branchLane = 0;
          while (occupied.has(branchLane)) branchLane += 1;
          arrivals[parentRow].push({ lane: branchLane, primary: false });
        }
        edges.push({
          from: entry.id,
          fromRow: row,
          fromLane: lane,
          to: parent.id,
          toRow: parentRow,
          draft: false,
        });
      } else if (
        parent.primary &&
        oldest !== undefined &&
        newest !== undefined
      ) {
        edges.push({
          from: entry.id,
          fromRow: row,
          fromLane: lane,
          boundary: base < oldest ? "top" : base > newest ? "bottom" : "top",
          draft: false,
        });
      }
    }
  }

  const draftCount = entries.filter((entry) => entry.draft).length;
  const draftIntervals = [
    ...edges.map((edge) => ({
      lane: edge.fromLane,
      start: Math.min(
        edge.fromRow,
        edge.toRow ?? (edge.boundary === "bottom" ? entries.length - 1 : 0),
      ),
      end: Math.max(
        edge.fromRow,
        edge.toRow ?? (edge.boundary === "bottom" ? entries.length - 1 : 0),
      ),
    })),
    ...nodes
      .filter((node) => !node.draft)
      .map((node) => ({ lane: node.lane, start: node.row, end: node.row })),
  ];
  for (const [row, entry] of entries.entries()) {
    if (!entry.draft) continue;
    const baseRow =
      entry.base !== undefined && /^\d+$/.test(entry.base)
        ? rows.get(`r${entry.base}`)
        : undefined;
    const isInline =
      baseRow !== undefined &&
      draftCount === 1 &&
      baseRow === published.at(-1)?.row;
    if (isInline) {
      nodes[row].lane = nodes[baseRow!].lane;
    } else {
      const endpoint = baseRow ?? (entry.base !== undefined ? 0 : row);
      const start = Math.min(row, endpoint);
      const end = Math.max(row, endpoint);
      let lane = 0;
      while (
        draftIntervals.some(
          (interval) =>
            interval.lane === lane &&
            interval.start <= end &&
            start <= interval.end,
        )
      ) {
        lane += 1;
      }
      nodes[row].lane = lane;
      draftIntervals.push({ lane, start, end });
    }
    if (baseRow !== undefined) {
      edges.push({
        from: entry.id,
        fromRow: row,
        fromLane: nodes[row].lane,
        to: `r${entry.base}`,
        toRow: baseRow,
        draft: true,
      });
    } else if (entry.base !== undefined && /^\d+$/.test(entry.base)) {
      const base = BigInt(entry.base);
      if (base === 0n && published.length === 0) continue;
      edges.push({
        from: entry.id,
        fromRow: row,
        fromLane: nodes[row].lane,
        boundary: oldest !== undefined && base > newest! ? "bottom" : "top",
        draft: true,
      });
    }
  }

  for (const edge of edges) {
    if (edge.toRow !== undefined) edge.toLane = nodes[edge.toRow].lane;
  }
  return {
    nodes,
    edges,
    laneCount: Math.max(1, ...nodes.map((node) => node.lane + 1)),
  };
}
