import { notFound, redirect } from "next/navigation";
import { session } from "@/lib/session";
import { Console } from "@/components/console";
import { rasterBasemap } from "@/lib/basemap";
export default async function Page({
  params,
  searchParams,
}: {
  params: Promise<{ section: string }>;
  searchParams: Promise<{ project?: string }>;
}) {
  const { section } = await params;
  if (
    ![
      "projects",
      "datasets",
      "workspaces",
      "history",
      "access",
      "audit",
      "service",
    ].includes(section)
  )
    notFound();
  if (!(await session()).token) redirect("/login");
  const query = await searchParams;
  const hasProject = Object.hasOwn(query, "project");
  return (
    <Console
      key={`${section}:${hasProject ? (query.project ?? "") : "missing"}`}
      section={section}
      projectId={hasProject ? (query.project ?? "") : undefined}
      basemap={rasterBasemap(process.env)}
    />
  );
}
