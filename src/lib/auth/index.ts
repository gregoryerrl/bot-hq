import crypto from "crypto";
import { db, authorizedDevices } from "@/lib/db";
import { eq, and } from "drizzle-orm";

export function generateToken(): string {
  return crypto.randomBytes(32).toString("hex");
}

export function hashToken(token: string): string {
  return crypto.createHash("sha256").update(token).digest("hex");
}

export function generatePairingCode(): string {
  return Math.floor(100000 + Math.random() * 900000).toString();
}

export async function verifyDevice(
  fingerprint: string,
  token: string
): Promise<boolean> {
  const tokenHash = hashToken(token);

  const device = await db
    .select()
    .from(authorizedDevices)
    .where(
      and(
        eq(authorizedDevices.deviceFingerprint, fingerprint),
        eq(authorizedDevices.tokenHash, tokenHash),
        eq(authorizedDevices.isRevoked, false)
      )
    )
    .limit(1);

  if (device.length > 0) {
    // Update last seen
    await db
      .update(authorizedDevices)
      .set({ lastSeenAt: new Date() })
      .where(eq(authorizedDevices.id, device[0].id));
    return true;
  }

  return false;
}

export async function authorizeDevice(
  deviceName: string,
  fingerprint: string
): Promise<string> {
  const token = generateToken();
  const tokenHash = hashToken(token);

  await db.insert(authorizedDevices).values({
    deviceName,
    deviceFingerprint: fingerprint,
    tokenHash,
  });

  return token;
}
