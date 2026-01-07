import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

// Routes that don't require authentication
const PUBLIC_ROUTES = [
  "/unauthorized",
  "/api/auth/register", // Register pending device
  "/api/auth/check",    // Check if device is authorized
  "/api/auth/poll",     // Poll for authorization status
];

// Routes that are only for admin (localhost)
const ADMIN_ONLY_ROUTES = [
  "/api/auth/pending",  // Get pending devices
  "/api/auth/approve",  // Approve device
  "/api/auth/reject",   // Reject device
];

export function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;
  const host = request.headers.get("host") || "";

  // Check if localhost (admin access)
  const isLocalhost = host.startsWith("localhost") ||
                      host.startsWith("127.0.0.1") ||
                      host.startsWith("[::1]");

  // Localhost always has full access
  if (isLocalhost) {
    return NextResponse.next();
  }

  // Block admin-only routes for non-localhost
  if (ADMIN_ONLY_ROUTES.some(route => pathname.startsWith(route))) {
    return NextResponse.json({ error: "Admin access only" }, { status: 403 });
  }

  // Allow public routes
  if (PUBLIC_ROUTES.some(route => pathname.startsWith(route))) {
    return NextResponse.next();
  }

  // Allow static files and Next.js internals
  if (
    pathname.startsWith("/_next") ||
    pathname.startsWith("/favicon") ||
    pathname.includes(".")
  ) {
    return NextResponse.next();
  }

  // Check for device token
  const deviceToken = request.cookies.get("device_token")?.value;
  const deviceId = request.cookies.get("device_id")?.value;

  if (!deviceToken || !deviceId) {
    // Redirect to unauthorized page
    const url = request.nextUrl.clone();
    url.pathname = "/unauthorized";
    return NextResponse.redirect(url);
  }

  // Token exists - let the API verify it (we can't do DB calls in edge middleware)
  // The actual verification happens in API routes via the auth lib
  // For now, presence of token is enough to proceed
  return NextResponse.next();
}

export const config = {
  matcher: [
    /*
     * Match all request paths except:
     * - _next/static (static files)
     * - _next/image (image optimization files)
     * - favicon.ico (favicon file)
     */
    "/((?!_next/static|_next/image|favicon.ico).*)",
  ],
};
