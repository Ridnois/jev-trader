import type { Metadata } from "next";
import { getRules } from "@/lib/v3/server";
import { RulesView } from "./view";

export const metadata: Metadata = { title: "Rules" };
export const dynamic = "force-dynamic";

export default function RulesPage() {
  return <RulesView rules={getRules()} />;
}
