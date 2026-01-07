import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { Sidebar } from "@/components/layout/sidebar";
import { Toaster } from "@/components/ui/sonner";
import { ChatPanel } from "@/components/chat-panel/chat-panel";

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
        <div className="flex h-screen">
          <Sidebar />
          <main className="flex-1 overflow-auto">{children}</main>
        </div>
        <ChatPanel />
        <Toaster />
      </body>
    </html>
  );
}
