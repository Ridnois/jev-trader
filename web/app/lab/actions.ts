"use server";

import { execFile } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { promisify } from "node:util";
import { revalidatePath } from "next/cache";
import { ROOT, forgetCaches, getLab, onPaper } from "@/lib/v3/server";

const run = promisify(execFile);
const BIN = path.join(ROOT, "target", "lab", "jev-trader");
const PID = path.join(ROOT, "data-v3", "trade.pid");

export type PromoteResult = { ok: boolean; message: string };

/** The family file whose `family = "…"` line names this id: hand-written under families/, generated under families/gen/. */
function familyFile(id: string): { file: string; name: string } | null {
  for (const dir of [path.join(ROOT, "families"), path.join(ROOT, "families", "gen")]) {
    let files: string[] = [];
    try {
      files = fs.readdirSync(dir).filter((f) => f.endsWith(".toml"));
    } catch {
      continue;
    }
    for (const f of files) {
      const text = fs.readFileSync(path.join(dir, f), "utf8");
      if (text.match(/^family\s*=\s*"([^"]+)"/m)?.[1] === id) {
        return { file: path.join(dir, f), name: text.match(/^name\s*=\s*"([A-Za-z0-9_-]+)"/m)?.[1] ?? id };
      }
    }
  }
  return null;
}

function alive(pid: number) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/**
 * Restart the paper trader so it loads strategies/ again. It saves its ledger, open positions
 * included, on Ctrl+C (SIGINT) and resumes them on start. A trader that was not running stays stopped.
 */
async function restartTrader(): Promise<string> {
  const pid = Number(fs.existsSync(PID) ? fs.readFileSync(PID, "utf8").trim() : NaN);
  if (!Number.isInteger(pid) || pid <= 0 || !alive(pid)) return "The trader is not running, so it will pick this up when you start it.";
  process.kill(pid, "SIGINT");
  for (let i = 0; i < 60 && alive(pid); i++) await new Promise((r) => setTimeout(r, 500));
  if (alive(pid)) return `The trader (pid ${pid}) did not stop within 30 s; restart it by hand to load the new strategy.`;
  try {
    await run(path.join(ROOT, "scripts", "v3-services.sh"), ["start", "trader"], { cwd: ROOT, timeout: 30_000 });
    return "The trader restarted with it; open paper positions were kept.";
  } catch (e) {
    return `The trader stopped but did not start again: ${(e as { stderr?: string }).stderr?.trim() || String(e)}. See data-v3/trade.log.`;
  }
}

/**
 * Hand a family to the paper trader: `lab promote`, as validated when the gate passed it,
 * otherwise with --candidate (labelled unproven). Then restart the trader.
 */
export async function promoteFamily(family: string, study: string): Promise<PromoteResult> {
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(family)) return { ok: false, message: "That is not a family id." };
  const lab = getLab();
  const report = [...lab.families, ...lab.generated].find((f) => f.family === family && f.study === study);
  if (!report) return { ok: false, message: `No verdict for ${family} on ${study}.` };
  const on = onPaper().find((p) => p.family === family);
  if (on) return { ok: false, message: `${family} is already on paper as ${on.name}.` };
  const found = familyFile(family);
  if (!found) return { ok: false, message: `No family file declares ${family}.` };
  // `lab promote` reads the verdict on `<research_venue>-<interval>`; make sure that is the one on screen.
  const venue = fs.readFileSync(path.join(ROOT, "lab.toml"), "utf8").match(/^research_venue\s*=\s*"([^"]+)"/m)?.[1] ?? "binance";
  const interval = fs.readFileSync(found.file, "utf8").match(/^interval\s*=\s*"([^"]+)"/m)?.[1] ?? "";
  if (`${venue}-${interval}` !== study) {
    return { ok: false, message: `${found.name} is judged on another study than ${study}; promote it from the command line.` };
  }
  if (!fs.existsSync(BIN)) return { ok: false, message: "target/lab/jev-trader is missing; build it with cargo build --profile lab -j 2." };
  const candidate = report.verdict !== "PROMOTE_TO_PAPER";
  const args = ["lab", "promote", "--family", path.relative(ROOT, found.file), "--as", found.name, ...(candidate ? ["--candidate"] : [])];
  try {
    await run(BIN, args, { cwd: ROOT, timeout: 180_000 });
  } catch (e) {
    const err = e as { stderr?: string; stdout?: string };
    const why = (err.stderr || err.stdout || String(e)).trim().split("\n").filter(Boolean).at(-1) ?? "unknown error";
    return { ok: false, message: `lab promote failed: ${why.replace(/\x1b\[[0-9;]*m/g, "")}` };
  }
  forgetCaches();
  const trader = await restartTrader();
  revalidatePath("/lab");
  revalidatePath("/paper");
  revalidatePath("/");
  return { ok: true, message: `${found.name} is on paper as ${candidate ? "an unproven candidate" : "validated"}. ${trader}` };
}
