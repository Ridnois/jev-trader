"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { Activity, BookOpen, CandlestickChart, FlaskConical, GitBranch, LineChart, Newspaper, ScanSearch } from "lucide-react";
import {
  Sidebar, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupContent, SidebarHeader, SidebarInset, SidebarMenu,
  SidebarMenuButton, SidebarMenuItem, SidebarProvider, SidebarTrigger, useSidebar,
} from "@/components/ui/sidebar";
import { engineState, useLive } from "@/components/live";
import { Dot } from "@/components/bits";
import { duration } from "@/lib/format";

const NAV: { href: string; label: string; icon: typeof Activity; station: "lab" | "paper" | "news" | "jev" | null }[] = [
  { href: "/", label: "Now", icon: Activity, station: null },
  { href: "/lab", label: "Lab", icon: FlaskConical, station: "lab" },
  { href: "/generations", label: "Generations", icon: GitBranch, station: null },
  { href: "/paper", label: "Paper run", icon: LineChart, station: "paper" },
  { href: "/catalysts", label: "News recorder", icon: Newspaper, station: "news" },
  { href: "/probes", label: "Jev tests", icon: ScanSearch, station: "jev" },
  { href: "/markets", label: "Markets", icon: CandlestickChart, station: null },
];

function Nav() {
  const pathname = usePathname();
  const { setOpenMobile } = useSidebar();
  return (
    <SidebarMenu className="gap-0.5">
      {NAV.map((n) => {
        const active = n.href === "/" ? pathname === "/" : pathname.startsWith(n.href);
        return (
          <SidebarMenuItem key={n.href}>
            <SidebarMenuButton asChild isActive={active} className="h-9 data-[active=true]:font-medium">
              <Link href={n.href} onClick={() => setOpenMobile(false)} aria-current={active ? "page" : undefined}>
                <n.icon strokeWidth={1.75} />
                <span>{n.label}</span>
              </Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
        );
      })}
    </SidebarMenu>
  );
}

function EngineFoot() {
  const { live } = useLive();
  const s = engineState(live);
  const started = live.engine.started ? Date.parse(live.engine.started) : null;
  return (
    <div className="rounded-lg border bg-panel p-3 text-[13px] leading-5">
      <div className="flex items-center gap-2 font-medium text-foreground">
        <Dot level={s.level} />
        {s.label}
      </div>
      <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 text-muted-foreground">
        {s.level !== "never" && started && s.level !== "stopped" && (
          <>
            <dt>Up</dt>
            <dd className="text-right text-foreground">{duration((live.now - started) / 1000)}</dd>
          </>
        )}
        <dt>Venue</dt>
        <dd className="text-right text-foreground">Hyperliquid</dd>
        <dt>Money</dt>
        <dd className="text-right text-foreground">Paper only</dd>
        <dt>Access</dt>
        <dd className="text-right text-foreground">Read-only</dd>
      </dl>
    </div>
  );
}

function Updated() {
  const { live, offline } = useLive();
  const s = engineState(live);
  if (offline)
    return (
      <span className="flex items-center gap-2 text-[13px] text-neg">
        <Dot level="stopped" />
        Dashboard server unreachable
      </span>
    );
  const ts = live.engine.status?.ts;
  const quiet = s.level === "running";
  return (
    <span className="flex items-center gap-2 text-[13px] text-muted-foreground">
      <Dot level={s.level} />
      <span className={quiet ? "md:hidden" : undefined}>{s.label}</span>
      {ts && (
        <span className={quiet ? "tabular-nums" : "text-faint tabular-nums"}>
          <span className={quiet ? "hidden md:inline" : "hidden sm:inline"}>{quiet ? "Last update " : "last status "}</span>
          {new Date(ts).toISOString().slice(11, quiet ? 19 : 16)} UTC
        </span>
      )}
    </span>
  );
}

export function Shell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const current = NAV.find((n) => (n.href === "/" ? pathname === "/" : pathname.startsWith(n.href)));
  return (
    <SidebarProvider>
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:border focus:bg-background focus:px-3 focus:py-1.5 focus:text-[13px]"
      >
        Skip to content
      </a>
      <Sidebar className="border-r">
        <SidebarHeader className="px-4 pt-5 pb-3">
          <Link href="/" className="flex items-center gap-2 rounded-sm text-[15px] font-semibold tracking-tight text-foreground">
            <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden className="shrink-0">
              <path d="M9 1.5 16.5 15.5h-15z" fill="currentColor" />
            </svg>
            jev-trader
            <span className="rounded-full border px-1.5 py-px text-[11px] font-medium text-muted-foreground">v3</span>
          </Link>
          <p className="mt-1 text-xs leading-4 text-balance text-muted-foreground">A research desk with a paper trading book</p>
        </SidebarHeader>
        <SidebarContent>
          <SidebarGroup className="px-2">
            <SidebarGroupContent>
              <Nav />
            </SidebarGroupContent>
          </SidebarGroup>
          <SidebarGroup className="mt-auto px-2 pb-2">
            <SidebarGroupContent>
              <SidebarMenu>
                <SidebarMenuItem>
                  <SidebarMenuButton asChild isActive={pathname.startsWith("/start")} className="h-9 data-[active=true]:font-medium">
                    <Link href="/start" aria-current={pathname.startsWith("/start") ? "page" : undefined}>
                      <BookOpen strokeWidth={1.75} />
                      <span>How this works</span>
                    </Link>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </SidebarContent>
        <SidebarFooter className="p-3">
          <EngineFoot />
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="min-w-0">
        <header className="sticky top-0 z-20 border-b bg-background/90 backdrop-blur">
          <div className="flex h-14 items-center gap-3 px-4 md:px-8">
            <SidebarTrigger className="-ml-1 md:hidden" />
            <h1 className="text-sm font-medium">{current?.label ?? (pathname.startsWith("/start") ? "How this works" : "jev-trader")}</h1>
            <div className="ml-auto">
              <Updated />
            </div>
          </div>
        </header>
        <div id="main" tabIndex={-1} className="mx-auto w-full max-w-[1320px] px-4 py-6 md:px-8 md:py-8">{children}</div>
      </SidebarInset>
    </SidebarProvider>
  );
}
