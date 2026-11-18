/**
 * Factory registry paging: `mapRawCirclePage` decoding and the
 * `FactoryClient.getCirclesPaginated` / `getAllCirclesPaged` helpers.
 *
 * The on-chain semantics these mirror are specified in
 * contracts/circle_factory/src/registry.rs.
 */

import { describe, it, expect, vi } from "vitest";
import { scValToNative, xdr } from "@stellar/stellar-sdk";
import { FactoryClient } from "../client";
import { mapRawCirclePage } from "../types";
import { FACTORY_MAX_PAGE_SIZE } from "../constants";
import { CIRCLE_ADDR, MEMBER_A_ADDR, MEMBER_B_ADDR, SDK_CONFIG } from "./fixtures";

/** A FactoryClient whose contract reads come from `pages[cursor]`. */
function clientWithRegistry(registry: string[], pageSize: number) {
  const client = new FactoryClient(SDK_CONFIG);
  const read = vi.fn(async (_id: string, method: string, args: unknown[]) => {
    expect(method).toBe("get_circles_paginated");
    const cursor = scValToNative(args[0] as xdr.ScVal) as number;
    const limit = Math.min(scValToNative(args[1] as xdr.ScVal) as number, pageSize);
    const end = Math.min(cursor + limit, registry.length);
    return {
      circles: registry.slice(cursor, end),
      next_cursor: end < registry.length ? end : null,
      total: registry.length,
    };
  });
  (client as unknown as { simulateAndReadOrThrow: typeof read }).simulateAndReadOrThrow = read;
  return { client, read };
}

describe("mapRawCirclePage", () => {
  it("decodes a middle page", () => {
    expect(
      mapRawCirclePage({ circles: [MEMBER_A_ADDR], next_cursor: 2, total: 3 }, 1),
    ).toEqual({ circles: [MEMBER_A_ADDR], nextCursor: 2, total: 3 });
  });

  it("maps a None cursor to undefined on the final page", () => {
    expect(mapRawCirclePage({ circles: [CIRCLE_ADDR], next_cursor: null, total: 1 }, 0)).toEqual({
      circles: [CIRCLE_ADDR],
      nextCursor: undefined,
      total: 1,
    });
  });

  it("accepts an empty page at cursor == total", () => {
    expect(mapRawCirclePage({ circles: [], next_cursor: undefined, total: 4 }, 4).nextCursor)
      .toBeUndefined();
  });

  it("rejects a cursor that would skip entries", () => {
    expect(() =>
      mapRawCirclePage({ circles: [CIRCLE_ADDR], next_cursor: 5, total: 9 }, 0),
    ).toThrow(/next_cursor 5 is inconsistent/);
  });

  it("rejects a final page that stops short of total", () => {
    expect(() =>
      mapRawCirclePage({ circles: [CIRCLE_ADDR], next_cursor: null, total: 9 }, 0),
    ).toThrow(/final page ends at 1/);
  });

  it("rejects a page larger than the on-chain maximum", () => {
    const tooMany = Array.from({ length: FACTORY_MAX_PAGE_SIZE + 1 }, () => CIRCLE_ADDR);
    expect(() =>
      mapRawCirclePage({ circles: tooMany, next_cursor: null, total: tooMany.length }, 0),
    ).toThrow(TypeError);
  });

  it("rejects a missing total", () => {
    expect(() => mapRawCirclePage({ circles: [] }, 0)).toThrow(/total/);
  });
});

describe("FactoryClient.getCirclesPaginated", () => {
  it("validates cursor and limit before any request", async () => {
    const { client, read } = clientWithRegistry([], FACTORY_MAX_PAGE_SIZE);
    await expect(client.getCirclesPaginated(-1, 10)).rejects.toThrow(RangeError);
    await expect(client.getCirclesPaginated(0, 0)).rejects.toThrow(RangeError);
    await expect(client.getCirclesPaginated(0, FACTORY_MAX_PAGE_SIZE + 1)).rejects.toThrow(
      RangeError,
    );
    expect(read).not.toHaveBeenCalled();
  });

  it("returns a typed page", async () => {
    const { client } = clientWithRegistry([MEMBER_A_ADDR, MEMBER_B_ADDR, CIRCLE_ADDR], 50);
    expect(await client.getCirclesPaginated(1, 1)).toEqual({
      circles: [MEMBER_B_ADDR],
      nextCursor: 2,
      total: 3,
    });
  });
});

describe("FactoryClient.getAllCirclesPaged", () => {
  const registry = [MEMBER_A_ADDR, MEMBER_B_ADDR, CIRCLE_ADDR, MEMBER_A_ADDR, MEMBER_B_ADDR];

  it("walks every page in creation order", async () => {
    const { client, read } = clientWithRegistry(registry, 50);
    const result = await client.getAllCirclesPaged({ limit: 2 });
    expect(result).toEqual({ circles: registry, nextCursor: undefined, resumeCursor: 5 });
    expect(read).toHaveBeenCalledTimes(3);
  });

  it("stops at maxPages and reports where to resume", async () => {
    const { client } = clientWithRegistry(registry, 50);
    const first = await client.getAllCirclesPaged({ limit: 2, maxPages: 1 });
    expect(first).toEqual({ circles: registry.slice(0, 2), nextCursor: 2, resumeCursor: 2 });
    const rest = await client.getAllCirclesPaged({ from: first.resumeCursor, limit: 2 });
    expect([...first.circles, ...rest.circles]).toEqual(registry);
  });

  it("resumes from total to read only circles appended later", async () => {
    const grown = [...registry];
    const { client } = clientWithRegistry(grown, 50);
    const before = await client.getAllCirclesPaged();
    grown.push(CIRCLE_ADDR);
    const after = await client.getAllCirclesPaged({ from: before.resumeCursor });
    expect(after.circles).toEqual([CIRCLE_ADDR]);
    expect(after.resumeCursor).toBe(6);
  });
});
