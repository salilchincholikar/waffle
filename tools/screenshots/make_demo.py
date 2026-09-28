#!/usr/bin/env python3
"""Demo workbooks for README and website screenshots (fictional data only).

Usage (from repo root):
    python3 -m venv "$VENV" && "$VENV/bin/pip" install xlsxwriter==3.2.9
    "$VENV/bin/python" tools/screenshots/make_demo.py      # writes build/demo/
"""
import os
import random

import xlsxwriter

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "..", "build", "demo")

PRODUCTS = [
    ("Ethiopia Yirgacheffe", "Single origin", 18.50, 7.40),
    ("Colombia Huila", "Single origin", 16.00, 6.10),
    ("Guatemala Antigua", "Single origin", 17.25, 6.90),
    ("Kenya AA", "Single origin", 21.00, 9.20),
    ("Sumatra Mandheling", "Single origin", 15.75, 6.30),
    ("House Espresso", "Blend", 14.00, 4.80),
    ("Morning Blend", "Blend", 12.50, 4.10),
    ("Decaf Swiss Water", "Decaf", 15.00, 6.60),
    ("Cold Brew Pack", "Seasonal", 19.00, 7.00),
    ("Holiday Roast", "Seasonal", 20.50, 7.80),
]
REGIONS = ["North", "South", "East", "West"]
MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]


def sales_workbook(path):
    rnd = random.Random(7)
    wb = xlsxwriter.Workbook(path)
    title = wb.add_format({"bold": True, "font_size": 18, "font_color": "#5B3A1E"})
    sub = wb.add_format({"font_color": "#8A6D55", "italic": True})
    money = wb.add_format({"num_format": "$#,##0.00"})
    money_bold = wb.add_format({"num_format": "$#,##0", "bold": True, "top": 2})
    pct = wb.add_format({"num_format": "0.0%"})
    pct_bold = wb.add_format({"num_format": "0.0%", "bold": True, "top": 2})
    int_bold = wb.add_format({"num_format": "#,##0", "bold": True, "top": 2})
    label_bold = wb.add_format({"bold": True, "top": 2})

    ws = wb.add_worksheet("Sales")
    ws.write("A1", "Roastery sales — 2026", title)
    ws.write("A2", "Units and revenue by product. Margin and totals are live formulas.", sub)
    ws.set_row(0, 30)
    headers = ["Product", "Type", "Region", "Units", "Price", "Cost", "Revenue", "Margin"]
    rows = []
    for name, kind, price, cost in PRODUCTS:
        for region in REGIONS[: 2 + rnd.randint(0, 2)]:
            rows.append((name, kind, region, rnd.randint(120, 1900), price, cost))
    first, last = 4, 4 + len(rows) - 1  # 1-based rows of the table body
    data = []
    for i, (name, kind, region, units, price, cost) in enumerate(rows):
        r = first + i
        data.append([name, kind, region, units, price, cost, f"=D{r}*E{r}", f"=IF(G{r}=0,0,(E{r}-F{r})/E{r})"])
    ws.add_table(first - 2, 0, last - 1, 7, {
        "name": "Sales",
        "style": "Table Style Medium 7",
        "columns": [{"header": h} for h in headers],
        "data": data,
    })
    for r in range(first - 1, last):
        ws.write_number(r, 4, data[r - first + 1][4], money)
        ws.write_number(r, 5, data[r - first + 1][5], money)
        ws.write_formula(r, 6, data[r - first + 1][6], money)
        ws.write_formula(r, 7, data[r - first + 1][7], pct)
    t = last + 1
    ws.write(t - 1, 0, "Total", label_bold)
    for c in (1, 2, 4, 5):
        ws.write_blank(t - 1, c, None, label_bold)
    ws.write_formula(t - 1, 3, f"=SUM(D{first}:D{last})", int_bold)
    ws.write_formula(t - 1, 6, f"=SUM(G{first}:G{last})", money_bold)
    ws.write_formula(t - 1, 7, f"=AVERAGE(H{first}:H{last})", pct_bold)
    ws.conditional_format(f"D{first}:D{last}", {"type": "data_bar", "bar_color": "#C8956B", "bar_solid": True})
    ws.conditional_format(f"H{first}:H{last}", {"type": "3_color_scale", "min_color": "#F8696B", "mid_color": "#FFEB84", "max_color": "#63BE7B"})
    ws.set_column("A:A", 24)
    ws.set_column("B:C", 13)
    ws.set_column("D:D", 10)
    ws.set_column("E:G", 12)
    ws.set_column("H:H", 10)
    ws.freeze_panes(first - 1, 1)
    ws.activate()

    by = wb.add_worksheet("By month")
    head = wb.add_format({"bold": True, "bg_color": "#5B3A1E", "font_color": "#FFFFFF", "align": "center"})
    by.write_row(0, 0, ["Region"] + MONTHS + ["Year"], head)
    for i, region in enumerate(REGIONS):
        by.write(i + 1, 0, region, wb.add_format({"bold": True}))
        for m in range(12):
            by.write_number(i + 1, m + 1, rnd.randint(8, 40) * 100, wb.add_format({"num_format": "#,##0"}))
        by.write_formula(i + 1, 13, f"=SUM(B{i + 2}:M{i + 2})", wb.add_format({"num_format": "#,##0", "bold": True}))
    by.conditional_format("B2:M5", {"type": "2_color_scale", "min_color": "#FFF4E8", "max_color": "#C8956B"})
    by.set_column("A:A", 10)
    by.set_column("B:N", 8)

    notes = wb.add_worksheet("Notes")
    notes.write("A1", "Demo data for screenshots. Every number is made up.", sub)
    wb.close()


def contacts_csv(path):
    """Messy CSV for the clean-up tools shot."""
    rnd = random.Random(11)
    first = ["  Ana", "BEN", "chloe ", "Dev", "Elena", "farid", "Grace  ", "HIRO", "ines", "Jonas"]
    last = ["Silva", "okafor", "PATEL", "  Kim", "Rossi", "Haddad", "lee", "Tanaka ", "Moreau", "berg"]
    cities = ["Lisbon", "lagos", "PUNE", "Seoul ", "Milan", " Beirut", "Toronto", "osaka", "Lyon", "Oslo"]
    with open(path, "w", newline="") as f:
        f.write("First name,Last name,City,Joined,Balance\n")
        for i in range(400):
            d = f"2026-{rnd.randint(1, 9):02d}-{rnd.randint(1, 28):02d}"
            bal = f"{rnd.randint(-900, 9000) / 10:.2f}"
            f.write(f"{rnd.choice(first)},{rnd.choice(last)},{rnd.choice(cities)},{d},{bal}\n")


def main():
    os.makedirs(OUT, exist_ok=True)
    sales_workbook(os.path.join(OUT, "Roastery Sales.xlsx"))
    contacts_csv(os.path.join(OUT, "contacts.csv"))
    print("wrote", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
