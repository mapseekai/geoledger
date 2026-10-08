import { Login } from "@/components/login";
import { connection } from "next/server";
export default async function Page({
  searchParams,
}: {
  searchParams: Promise<{ expired?: string }>;
}) {
  await connection();
  return <Login expired={(await searchParams).expired === "1"} />;
}
