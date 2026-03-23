import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { Sidebar } from "@/components/layout/sidebar";
import { MobileNav } from "@/components/layout/mobile-nav";
import { Toaster } from "@/components/ui/sonner";
import { CommandContextProvider } from "@/components/command-bar/command-context";
import { CommandBar } from "@/components/command-bar/command-bar";

const inter = Inter({ subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Bot-HQ",
  description: "Workflow Automation System",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className={inter.className}>
        <CommandContextProvider>
          <div className="flex h-screen">
            <Sidebar />
            <MobileNav />
            <main className="flex-1 overflow-auto flex flex-col">
              <CommandBar />
              <div className="flex-1 overflow-auto">{children}</div>
            </main>
          </div>
          <Toaster />
        </CommandContextProvider>
      </body>
    </html>
  );
}
