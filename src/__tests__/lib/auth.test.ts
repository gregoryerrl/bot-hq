import { describe, it, expect } from "vitest";
import { generateToken, hashToken, generatePairingCode } from "@/lib/auth";

describe("generateToken", () => {
  it("generates a 64-character hex string", () => {
    const token = generateToken();
    expect(token).toHaveLength(64);
    expect(/^[a-f0-9]+$/.test(token)).toBe(true);
  });

  it("generates unique tokens", () => {
    const token1 = generateToken();
    const token2 = generateToken();
    expect(token1).not.toBe(token2);
  });
});

describe("hashToken", () => {
  it("returns consistent hash for same input", () => {
    const token = "test-token";
    const hash1 = hashToken(token);
    const hash2 = hashToken(token);
    expect(hash1).toBe(hash2);
  });

  it("returns different hash for different input", () => {
    const hash1 = hashToken("token1");
    const hash2 = hashToken("token2");
    expect(hash1).not.toBe(hash2);
  });

  it("returns a 64-character hex string (SHA256)", () => {
    const hash = hashToken("test");
    expect(hash).toHaveLength(64);
    expect(/^[a-f0-9]+$/.test(hash)).toBe(true);
  });
});

describe("generatePairingCode", () => {
  it("generates a 6-digit numeric string", () => {
    const code = generatePairingCode();
    expect(code).toHaveLength(6);
    expect(/^\d{6}$/.test(code)).toBe(true);
  });

  it("generates codes between 100000 and 999999", () => {
    for (let i = 0; i < 100; i++) {
      const code = parseInt(generatePairingCode(), 10);
      expect(code).toBeGreaterThanOrEqual(100000);
      expect(code).toBeLessThanOrEqual(999999);
    }
  });
});
