import { getLive } from "@/lib/v3/server";

export const dynamic = "force-dynamic";

export function GET() {
  return Response.json(getLive(), { headers: { "cache-control": "no-store" } });
}
