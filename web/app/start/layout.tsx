import type { Metadata } from "next";

export const metadata: Metadata = { title: "How this works" };

export default function StartLayout({ children }: { children: React.ReactNode }) {
  return children;
}
