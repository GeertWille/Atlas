import { expect, test } from "vitest";
import { makeProxy } from "@/test/fixtures";
import {
  hasMissingProxy,
  proxyAddress,
  proxyChoiceFromKey,
  proxyChoiceKey,
  proxyChoiceLabel,
  proxyUseLabel,
} from "./proxy";

const belgium = makeProxy();

test("a proxy choice survives the round trip through a select value", () => {
  for (const choice of [{ kind: "none" }, { kind: "system" }, { kind: "profile", id: "proxy-be" }] as const) {
    expect(proxyChoiceFromKey(proxyChoiceKey(choice))).toEqual(choice);
  }
  // The name and host snapshot are the backend's to fill in: they never go back as input.
  expect(proxyChoiceFromKey(proxyChoiceKey({ kind: "profile", id: "proxy-be", name: "Belgium", host: "be" }))).toEqual({
    kind: "profile",
    id: "proxy-be",
  });
});

test("a choice is named after the live profile, and after its snapshot once the profile is gone", () => {
  expect(proxyChoiceLabel({ kind: "none" }, [])).toBe("No proxy");
  expect(proxyChoiceLabel({ kind: "system" }, [])).toBe("System proxy");
  expect(proxyChoiceLabel({ kind: "profile", id: "proxy-be", name: "Old name" }, [belgium])).toBe("Belgium");
  expect(proxyChoiceLabel({ kind: "profile", id: "proxy-de", name: "Germany" }, [belgium])).toBe("Germany");
  expect(proxyChoiceLabel({ kind: "profile", id: "proxy-de" }, [belgium])).toBe("Deleted proxy");
});

test("the proxy of a finished scan reads with its host and never with credentials", () => {
  expect(proxyUseLabel({ kind: "none", name: null, host: null })).toBe("No proxy");
  expect(proxyUseLabel({ kind: "profile", name: "Belgium", host: "be.proxy.example" })).toBe(
    "Belgium (be.proxy.example)",
  );
  expect(proxyUseLabel({ kind: "profile", name: null, host: null })).toBe("Proxy");
  expect(proxyAddress(makeProxy({ username: "scanner", hasPassword: true }))).toBe("http://be.proxy.example:8080");
  expect(proxyAddress({ scheme: "socks5", host: "2001:db8::1", port: 1080 })).toBe("socks5://[2001:db8::1]:1080");
});

test("options that name a deleted profile are missing their proxy; everything else is not", () => {
  const settings = { proxies: [belgium] };
  expect(hasMissingProxy({ proxy: { kind: "profile", id: "proxy-de" } }, settings)).toBe(true);
  expect(hasMissingProxy({ proxy: { kind: "profile", id: "proxy-be" } }, settings)).toBe(false);
  expect(hasMissingProxy({ proxy: { kind: "system" } }, settings)).toBe(false);
  expect(hasMissingProxy({}, settings)).toBe(false);
});
