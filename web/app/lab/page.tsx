import type { Metadata } from "next";
import { getLab } from "@/lib/v3/server";
import { LabView } from "./view";

export const metadata: Metadata = { title: "Lab" };
export const dynamic = "force-dynamic";

export default function LabPage() {
  return <LabView lab={getLab()} />;
}
