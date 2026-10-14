import { notFound, redirect } from "next/navigation";
import { GeoLedgerError } from "@geoledger/client";
import { authRequest } from "@/lib/console-client";
import type { User } from "@/lib/auth-model";
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
      "project-status",
      "api-keys",
      "users",
      "profile",
      "password",
      "audit",
      "service",
    ].includes(section)
  )
    notFound();
  const auth = await session();
  if (!auth.sid) redirect("/login");
  let user: User;
  try {
    user = await authRequest<User>("me", {}, auth.sid);
  } catch (error) {
    if (error instanceof GeoLedgerError && error.code === "unauthenticated")
      redirect("/login?expired=1");
    throw error;
  }
  if (user.must_change_password && section !== "password")
    redirect("/password");
  if (section === "users" && !user.is_admin) notFound();
  const query = await searchParams;
  const hasProject = Object.hasOwn(query, "project");
  return (
    <Console
      key={`${section}:${hasProject ? (query.project ?? "") : "missing"}`}
      section={section}
      user={user}
      projectId={hasProject ? (query.project ?? "") : undefined}
      basemap={rasterBasemap(process.env)}
    />
  );
}
