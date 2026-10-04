// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import {type DataColumn, DataTable} from "./data-table";

type Row = { id: string; n: number };
const columns: DataColumn<Row>[] = [{ accessorKey: "n", header: "N" }];
const rows: Row[] = [
  { id: "a", n: 1 },
  { id: "b", n: 2 },
];

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});
afterEach(() => {
  document.body.innerHTML = "";
});

describe("DataTable onRowContextMenu (#601)", () => {
  it("hands the right-clicked row to the handler", () => {
    const onRowContextMenu = vi.fn();
    const host = document.createElement("div");
    document.body.appendChild(host);
    act(() => {
      createRoot(host).render(
        <DataTable columns={columns} data={rows} getRowId={(r) => r.id} onRowContextMenu={onRowContextMenu} />,
      );
    });
    const second = host.querySelectorAll("tbody tr")[1];
    act(() => {
      second.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    });
    expect(onRowContextMenu).toHaveBeenCalledTimes(1);
    expect(onRowContextMenu.mock.calls[0][0]).toEqual({ id: "b", n: 2 });
  });
});
