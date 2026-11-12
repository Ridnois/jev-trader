"use client";

import { createContext, useContext, useEffect, useState } from "react";
import type { Live } from "@/lib/v3/types";

type Ctx = { live: Live; /** The dashboard's own server stopped answering. */ offline: boolean };
const LiveContext = createContext<Ctx | null>(null);

const POLL_MS = 2000;

export function LiveProvider({ initial, children }: { initial: Live; children: React.ReactNode }) {
  const [live, setLive] = useState(initial);
  const [offline, setOffline] = useState(false);

  useEffect(() => {
    let stop = false;
    let timer: ReturnType<typeof setTimeout>;
    const tick = async () => {
      if (!document.hidden) {
        try {
          const r = await fetch("/api/live", { cache: "no-store" });
          if (!r.ok) throw new Error(String(r.status));
          const next = (await r.json()) as Live;
          if (!stop) {
            setLive(next);
            setOffline(false);
          }
        } catch {
          if (!stop) setOffline(true);
        }
      }
      if (!stop) timer = setTimeout(tick, POLL_MS);
    };
    timer = setTimeout(tick, POLL_MS);
    const wake = () => {
      if (!document.hidden) {
        clearTimeout(timer);
        tick();
      }
    };
    document.addEventListener("visibilitychange", wake);
    return () => {
      stop = true;
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", wake);
    };
  }, []);

  return <LiveContext.Provider value={{ live, offline }}>{children}</LiveContext.Provider>;
}

export function useLive(): Ctx {
  const c = useContext(LiveContext);
  if (!c) throw new Error("useLive outside LiveProvider");
  return c;
}

export { engineState } from "@/lib/v3/stations";
export type { EngineState } from "@/lib/v3/stations";
