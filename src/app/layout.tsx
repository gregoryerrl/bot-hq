import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { Sidebar } from "@/components/layout/sidebar";
import { MobileNav } from "@/components/layout/mobile-nav";
import { Toaster } from "@/components/ui/sonner";
import { ChatPanel } from "@/components/chat-panel/chat-panel";
import { NotificationProvider } from "@/components/notifications/notification-provider";
import { AwaitingInputBanner } from "@/components/notifications/awaiting-input-banner";

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
        <NotificationProvider>
          <div className="flex h-screen">
            <Sidebar />
            <MobileNav />
            <main className="flex-1 overflow-auto flex flex-col">
              <AwaitingInputBanner />
              <div className="flex-1 overflow-auto">{children}</div>
            </main>
          </div>
          <ChatPanel />
          <Toaster />
        </NotificationProvider>
      </body>
    </html>
  );
}
