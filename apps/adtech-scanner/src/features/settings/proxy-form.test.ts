import { expect, test } from "vitest";
import { makeProxy } from "@/test/fixtures";
import { newProxyForm, type ProxyForm, proxyFormOf, validateProxyForm, withScheme } from "./proxy-form";

const form = (patch: Partial<ProxyForm> = {}): ProxyForm => ({
  ...newProxyForm(),
  name: "Belgium",
  host: "be.proxy.example",
  ...patch,
});

test("a complete form becomes a draft, trimmed, with the password kept apart", () => {
  expect(
    validateProxyForm(
      form({ name: " Belgium ", host: " be.proxy.example ", username: " scanner ", password: " s3cret " }),
      [],
    ),
  ).toEqual({
    ok: true,
    draft: { name: "Belgium", scheme: "http", host: "be.proxy.example", port: 8080, username: "scanner" },
    // A password is taken as typed: spaces can be part of it.
    password: " s3cret ",
  });
  expect(validateProxyForm(form(), [], "proxy-be")).toEqual({
    ok: true,
    draft: { id: "proxy-be", name: "Belgium", scheme: "http", host: "be.proxy.example", port: 8080, username: null },
    password: null,
  });
});

test.each([
  ["no name", { name: " " }, "name", "Give the proxy a name, like the country it is in."],
  ["no host", { host: "" }, "host", "Enter the proxy's host name or IP address."],
  ["a URL as host", { host: "http://be.proxy.example" }, "host", /no http:\/\/, port, path or password/],
  ["a host with a port", { host: "be.proxy.example:8080" }, "host", /Only the host name or IP address/],
  ["a host with a path", { host: "be.proxy.example/x" }, "host", /Only the host name or IP address/],
  ["an IP address with a port", { host: "10.0.0.7:8080" }, "host", /Only the host name or IP address/],
  ["a hex-looking host with a port", { host: "cafe.de:8080" }, "host", /Only the host name or IP address/],
  ["credentials in the host", { host: "me:pw@be.proxy.example" }, "host", /Only the host name or IP address/],
  ["a port of zero", { port: "0" }, "port", "A port from 1 to 65535."],
  ["a port that is not a number", { port: "http" }, "port", "A port from 1 to 65535."],
  ["a port above the range", { port: "65536" }, "port", "A port from 1 to 65535."],
  ["a password without a user name", { password: "pw" }, "username", "Enter the user name this password belongs to."],
] as const)("%s is explained on its field", (_name, patch, field, message) => {
  const result = validateProxyForm(form(patch), []);
  expect(result.ok).toBe(false);
  expect(!result.ok && Object.keys(result.errors)).toEqual([field]);
  expect(!result.ok && result.errors[field]).toMatch(message);
});

test("SOCKS5 with a user name is refused with the reason and the way out", () => {
  const result = validateProxyForm(form({ scheme: "socks5", username: "scanner", password: "pw" }), []);
  expect(result).toEqual({
    ok: false,
    errors: {
      username: "Chrome cannot sign in to a SOCKS5 proxy. Remove the user name, or use an HTTP or HTTPS proxy.",
    },
  });
  expect(validateProxyForm(form({ scheme: "socks5" }), []).ok).toBe(true);
});

test("a name another proxy has is refused whatever its case; the proxy's own name is not", () => {
  expect(validateProxyForm(form({ name: "belgium" }), ["Belgium", "France"])).toMatchObject({
    ok: false,
    errors: { name: "A proxy named “belgium” already exists." },
  });
  expect(validateProxyForm(form(), ["France"], "proxy-be").ok).toBe(true);
});

test("IP addresses are hosts too", () => {
  for (const host of ["10.0.0.7", "2001:db8::1", "localhost"]) {
    expect(validateProxyForm(form({ host }), []).ok, host).toBe(true);
  }
});

test("several problems are reported at once", () => {
  const result = validateProxyForm(form({ name: "", host: "", port: "" }), []);
  expect(!result.ok && Object.keys(result.errors).sort()).toEqual(["host", "name", "port"]);
});

test("the scheme brings its usual port, unless the user typed their own", () => {
  expect(withScheme(newProxyForm(), "socks5").port).toBe("1080");
  expect(withScheme(withScheme(newProxyForm(), "socks5"), "https").port).toBe("443");
  expect(withScheme({ ...newProxyForm(), port: "3128" }, "socks5").port).toBe("3128");
  expect(withScheme({ ...newProxyForm(), port: " " }, "https").port).toBe("443");
});

test("editing starts from the saved proxy and never from its password", () => {
  expect(proxyFormOf(makeProxy({ username: "scanner", hasPassword: true }))).toEqual({
    name: "Belgium",
    scheme: "http",
    host: "be.proxy.example",
    port: "8080",
    username: "scanner",
    password: "",
  });
  expect(proxyFormOf(makeProxy()).username).toBe("");
});

test.each([
  ["10.0.0.7", "10.0.0.7"],
  ["2001:db8::1", "2001:db8::1"],
  ["::1", "::1"],
  // Brackets are how an IPv6 address is written in a URL; the backend stores the bare address.
  ["[2001:db8::1]", "2001:db8::1"],
])("the address %s is accepted as a host", (typed, stored) => {
  const result = validateProxyForm(form({ host: typed }), []);
  expect(result.ok && result.draft.host).toBe(stored);
});
