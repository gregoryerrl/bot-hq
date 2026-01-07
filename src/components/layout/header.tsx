import { NotificationBell } from "@/components/notifications/notification-bell";

interface HeaderProps {
  title: string;
  description?: string;
}

export function Header({ title, description }: HeaderProps) {
  return (
    <header className="border-b px-4 md:px-6 py-4">
      <div className="flex items-center justify-between">
        <div className="ml-10 md:ml-0">
          <h1 className="text-xl md:text-2xl font-semibold">{title}</h1>
          {description && (
            <p className="text-xs md:text-sm text-muted-foreground">{description}</p>
          )}
        </div>
        <NotificationBell />
      </div>
    </header>
  );
}
