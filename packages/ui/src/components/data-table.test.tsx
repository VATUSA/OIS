import * as React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";

import {type DataColumn, DataTable} from "./data-table";

type Row = { id: string; n: number };

const columns: DataColumn<Row>[] = [{ accessorKey: "n", header: "N" }];
const rows: Row[] = [
  { id: "a", n: 2 },
  { id: "b", n: 3 },
  { id: "c", n: 1 },
];

/** Whether the "N" column header's sort button is enabled. */
function headerSortable(html: string): boolean {
  const button = /<th[^>]*>(<button[^>]*>)/.exec(html)?.[1] ?? "";
  return !/\sdisabled/.test(button);
}

/** The rendered cell values, top to bottom. */
function order(html: string): string[] {
  return [...html.matchAll(/<td[^>]*>(?:<[^>]+>)*(\d+)/g)].map((m) => m[1]);
}

describe("DataTable sorting", () => {
  it("sorts client-side data", () => {
    const html = renderToStaticMarkup(
      <DataTable columns={columns} data={rows} getRowId={(r) => r.id} initialSort={[{ id: "n", desc: false }]} />,
    );
    expect(order(html)).toEqual(["1", "2", "3"]);
    expect(headerSortable(html)).toBe(true);
  });

  // A server page is one slice of a larger ordered set: re-sorting it client-side would present
  // "the lowest of these 25" as if it were the lowest overall, so sorting is off.
  it("keeps the API's order and offers no sort on a server-paged table", () => {
    const html = renderToStaticMarkup(
      <DataTable
        columns={columns}
        data={rows}
        getRowId={(r) => r.id}
        initialSort={[{ id: "n", desc: false }]}
        serverPagination={{ page: 1, pageSize: 3, total: 30, onPageChange: () => {} }}
      />,
    );
    expect(order(html)).toEqual(["2", "3", "1"]);
    expect(headerSortable(html)).toBe(false);
    expect(html).not.toContain("aria-sort");
  });
});
