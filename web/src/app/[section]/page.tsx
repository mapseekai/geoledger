import { notFound, redirect } from "next/navigation";
import { session } from "@/lib/session";
import { Console } from "@/components/console";
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
  return (
    <Console
      key={`${section}:${(await searchParams).project ?? ""}`}
      section={section}
      projectId={(await searchParams).project ?? ""}
    />
  );
}
