import type { Metadata } from "next";

export const metadata: Metadata = { title: "Paper run" };

export default function PaperLayout({ children }: { children: React.ReactNode }) {
  return children;
}
