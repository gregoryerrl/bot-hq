import { generatePairingCode } from "./index";

interface PendingPairing {
  code: string;
  deviceName: string;
  fingerprint: string;
  expiresAt: Date;
}

// In-memory store for pending pairings (cleared on restart)
const pendingPairings = new Map<string, PendingPairing>();

// Current active pairing code (displayed on Mac)
let activePairingSession: {
  code: string;
  expiresAt: Date;
} | null = null;

export function createPairingSession(): string {
  const code = generatePairingCode();
  activePairingSession = {
    code,
    expiresAt: new Date(Date.now() + 5 * 60 * 1000), // 5 minutes
  };
  return code;
}

export function getActivePairingCode(): string | null {
  if (!activePairingSession) return null;
  if (new Date() > activePairingSession.expiresAt) {
    activePairingSession = null;
    return null;
  }
  return activePairingSession.code;
}

export function requestPairing(
  code: string,
  deviceName: string,
  fingerprint: string
): boolean {
  if (!activePairingSession || code !== activePairingSession.code) {
    return false;
  }
  if (new Date() > activePairingSession.expiresAt) {
    activePairingSession = null;
    return false;
  }

  pendingPairings.set(fingerprint, {
    code,
    deviceName,
    fingerprint,
    expiresAt: new Date(Date.now() + 2 * 60 * 1000), // 2 minutes to approve
  });

  return true;
}

export function getPendingPairings(): PendingPairing[] {
  const now = new Date();
  const valid: PendingPairing[] = [];

  for (const [key, pairing] of pendingPairings) {
    if (now > pairing.expiresAt) {
      pendingPairings.delete(key);
    } else {
      valid.push(pairing);
    }
  }

  return valid;
}

export function approvePairing(fingerprint: string): PendingPairing | null {
  const pairing = pendingPairings.get(fingerprint);
  if (!pairing) return null;

  pendingPairings.delete(fingerprint);
  activePairingSession = null; // Clear after successful pairing

  return pairing;
}

export function rejectPairing(fingerprint: string): void {
  pendingPairings.delete(fingerprint);
}
