import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";
import { getLive } from "@/lib/v3/server";
import { LiveProvider } from "@/components/live";
import { Shell } from "@/components/shell";
import { TooltipProvider } from "@/components/ui/tooltip";

const geistSans = Geist({ variable: "--font-geist-sans", subsets: ["latin"] });
const geistMono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] });

export const metadata: Metadata = {
  title: { default: "jev-trader v3", template: "%s · jev-trader v3" },
  description: "Paper trader, catalyst recorder and research lab for jev-trader v3.",
};

export const dynamic = "force-dynamic";

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${geistSans.variable} ${geistMono.variable} h-full antialiased`}>
      <body className="min-h-full">
        <LiveProvider initial={getLive()}>
          <TooltipProvider delayDuration={150}>
            <Shell>{children}</Shell>
          </TooltipProvider>
        </LiveProvider>
      </body>
    </html>
  );
}
