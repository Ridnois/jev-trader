import type { Metadata } from "next";
import { getGenerations } from "@/lib/v3/server";
import { GenerationsView } from "./view";

export const metadata: Metadata = { title: "Generations" };
export const dynamic = "force-dynamic";

export default function GenerationsPage() {
  return <GenerationsView gens={getGenerations()} />;
}
