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


def overview_workbook(path):
    """A minimal Q3 dashboard: KPIs, a monthly table with soft colour, no gridlines."""
    rnd = random.Random(3)
    wb = xlsxwriter.Workbook(path)
    ink, muted, green, red, line = "#12301F", "#7A857E", "#2F7D3B", "#B23A2E", "#D9DED9"
    f = lambda **k: wb.add_format({"font_name": "Helvetica Neue", "font_color": ink, "valign": "vcenter", **k})
    title = f(bold=True, font_size=22)
    sub = f(font_color=muted, font_size=12)
    kpi_label = f(font_color=muted, font_size=11)
    kpi_value = f(bold=True, font_size=24, align="left", num_format="$#,##0", bottom=5, bottom_color="#EE8B2F")
    kpi_value_n = f(bold=True, font_size=24, align="left", num_format="#,##0", bottom=5, bottom_color="#EE8B2F")
    kpi_value_p = f(bold=True, font_size=24, align="left", num_format="0.0%", bottom=5, bottom_color="#EE8B2F")
    kpi_delta = f(font_color=green, font_size=11, bold=True)
    head = f(bold=True, font_color=muted, font_size=11, bottom=1, bottom_color=line)
    head_r = f(bold=True, font_color=muted, font_size=11, bottom=1, bottom_color=line, align="right")
    name = f(font_size=13)
    kind = f(font_size=12, font_color=muted)
    money = f(font_size=13, num_format="$#,##0")
    money_b = f(font_size=13, num_format="$#,##0", bold=True)
    pct = f(font_size=13, num_format="+0.0%;-0.0%")
    share = f(font_size=12, num_format="0%", font_color=muted)
    tot_l = f(bold=True, font_size=13, top=1, top_color=ink)
    tot_m = f(bold=True, font_size=13, num_format="$#,##0", top=1, top_color=ink)
    tot_p = f(bold=True, font_size=13, num_format="+0.0%;-0.0%", top=1, top_color=ink)
    tot_s = f(bold=True, font_size=12, num_format="0%", top=1, top_color=ink)

    ws = wb.add_worksheet("Overview")
    ws.set_column("A:A", 3)
    ws.set_column("B:B", 24)
    ws.set_column("C:C", 14)
    ws.set_column("D:G", 13)
    ws.set_column("H:I", 11)
    ws.set_row(1, 34)
    ws.write("B2", "Roastery — Q3 overview", title)
    ws.write("B3", "July to September 2026 · all figures are live formulas", sub)

    # Product rows first (the KPIs reference them).
    rows = []
    for n, k, price, _ in PRODUCTS[:8]:
        base = rnd.randint(9, 30) * 1000
        rows.append((n, k, [round(base * (1 + rnd.uniform(-0.12, 0.28) * i / 2)) for i in range(3)]))
    top = 11
    ws.write_row(top - 1, 1, ["Product", "Type"], head)
    ws.write_row(top - 1, 3, ["Jul", "Aug", "Sep", "Q3 total"], head_r)
    ws.write_row(top - 1, 7, ["Growth", "Share"], head_r)
    last = top + len(rows) - 1
    for i, (n, k, m) in enumerate(rows):
        r = top + i
        ws.set_row(r, 24)
        ws.write(r, 1, n, name)
        ws.write(r, 2, k, kind)
        for j, v in enumerate(m):
            ws.write_number(r, 3 + j, v, money)
        x = r + 1
        ws.write_formula(r, 6, f"=SUM(D{x}:F{x})", money_b)
        ws.write_formula(r, 7, f"=(F{x}-D{x})/D{x}", pct)
        ws.write_formula(r, 8, f"=G{x}/SUM($G${top + 1}:$G${last + 1})", share)
    t = last + 1
    ws.set_row(t, 26)
    ws.write(t, 1, "Total", tot_l)
    ws.write_blank(t, 2, None, tot_l)
    for j, col in enumerate("DEFG"):
        ws.write_formula(t, 3 + j, f"=SUM({col}{top + 1}:{col}{last + 1})", tot_m)
    ws.write_formula(t, 7, f"=(F{t + 1}-D{t + 1})/D{t + 1}", tot_p)
    ws.write_formula(t, 8, f"=SUM(I{top + 1}:I{last + 1})", tot_s)
    rng = f"D{top + 1}:F{last + 1}"
    ws.conditional_format(rng, {"type": "2_color_scale", "min_color": "#FFFFFF", "max_color": "#CFE6C4"})
    ws.conditional_format(f"H{top + 1}:H{last + 1}", {"type": "cell", "criteria": ">=", "value": 0, "format": wb.add_format({"font_color": green, "bold": True})})
    ws.conditional_format(f"H{top + 1}:H{last + 1}", {"type": "cell", "criteria": "<", "value": 0, "format": wb.add_format({"font_color": red, "bold": True})})
    ws.conditional_format(f"I{top + 1}:I{last + 1}", {"type": "data_bar", "bar_color": "#93C87D", "bar_solid": True, "bar_no_border": True})

    # KPIs: label, big number, change.
    ws.set_row(4, 20)
    ws.set_row(5, 38)
    ws.set_row(6, 20)
    kpis = [("B", "Revenue", f"=G{t + 1}", kpi_value, "▲ 18.4% vs Q2"),
            ("D", "Orders", "=3214", kpi_value_n, "▲ 9.1% vs Q2"),
            ("F", "Avg. order", f"=G{t + 1}/3214", kpi_value, "▲ 8.5% vs Q2"),
            ("H", "Growth", f"=H{t + 1}", kpi_value_p, "Jul → Sep")]
    for col, label, formula, fmt, delta in kpis:
        c2 = chr(ord(col) + 1)
        ws.merge_range(f"{col}5:{c2}5", label, kpi_label)
        ws.merge_range(f"{col}6:{c2}6", "", fmt)
        ws.write_formula(f"{col}6", formula, fmt)
        ws.merge_range(f"{col}7:{c2}7", delta, kpi_delta)
    ws.activate()

    # Orders: the detail, plainly styled.
    od = wb.add_worksheet("Orders")
    oh = f(bold=True, font_color=muted, bottom=1, bottom_color=line)
    od.write_row(0, 0, ["Order", "Date", "Product", "Region", "Units", "Amount"], oh)
    for i in range(120):
        n, _, price, _ = rnd.choice(PRODUCTS)
        u = rnd.randint(1, 40)
        od.write_row(i + 1, 0, [f"#{4100 + i}", f"2026-0{rnd.randint(7, 9)}-{rnd.randint(1, 28):02d}", n, rnd.choice(REGIONS), u], f(font_size=12))
        od.write_number(i + 1, 5, u * price, f(font_size=12, num_format="$#,##0.00"))
    od.set_column("A:B", 12)
    od.set_column("C:C", 22)
    od.set_column("D:F", 11)
    od.freeze_panes(1, 0)
    wb.add_worksheet("Notes").write("A1", "Demo data for screenshots. Every number is made up.", sub)
    wb.close()


TASKS = [
    # task, owner, status, priority, due, budget, spent, progress
    ("Sign the lease", "Maya", "Done", "High", "2026-07-02", 4800, 4800, 1.0),
    ("Espresso machine & grinder", "Leo", "Done", "High", "2026-07-15", 12500, 11890, 1.0),
    ("Interior design & fit-out", "Maya", "In progress", "High", "2026-08-20", 18000, 19240, 0.8),
    ("Hire baristas (3)", "Priya", "In progress", "Medium", "2026-08-25", 2400, 950, 0.5),
    ("Menu & pricing", "Leo", "Done", "Medium", "2026-08-01", 600, 540, 1.0),
    ("Bean supplier contract", "Priya", "Done", "High", "2026-07-28", 3200, 3200, 1.0),
    ("Point-of-sale setup", "Sam", "In progress", "Medium", "2026-09-05", 1800, 720, 0.4),
    ("Health inspection", "Maya", "Blocked", "High", "2026-09-10", 350, 0, 0.1),
    ("Website & online orders", "Sam", "In progress", "Low", "2026-09-12", 2200, 1100, 0.6),
    ("Signage", "Leo", "Not started", "Medium", "2026-09-15", 1500, 0, 0.0),
    ("Launch party", "Priya", "Not started", "Low", "2026-09-26", 1200, 0, 0.0),
    ("Soft opening week", "Everyone", "Not started", "High", "2026-09-28", 900, 0, 0.0),
]


def launch_board(path):
    """A friendly project board: title band, summary chips, status pills, progress bars."""
    wb = xlsxwriter.Workbook(path)
    FONT = "Avenir Next"
    ink, muted, band, cream, stripe, line = "#12301F", "#6B7A70", "#1F5B33", "#FDF1DA", "#FBF8F2", "#EDE6D8"
    def f(**k):
        return wb.add_format({"font_name": FONT, "font_size": 12, "font_color": ink, "valign": "vcenter", **k})
    ws = wb.add_worksheet("Launch plan")
    widths = {"A": 2.5, "B": 34, "C": 11, "D": 14, "E": 11, "F": 12, "G": 12, "H": 12, "I": 16, "J": 2.5}
    for col, w in widths.items():
        ws.set_column(f"{col}:{col}", w)

    # Title band
    title = f(bold=True, font_size=22, font_color="#FFFFFF", bg_color=band)
    subtitle = f(font_size=12, font_color="#CFE6C4", bg_color=band)
    bandf = f(bg_color=band)
    for r, h in [(0, 14), (1, 40), (2, 22), (3, 14)]:
        ws.set_row(r, h)
        for c in range(0, 10):
            ws.write_blank(r, c, None, bandf)
    ws.write("B2", "☕  Harbor Street Café — launch plan", title)
    ws.write("B3", "Opening 28 September 2026 · 12 tasks · budget and progress update live", subtitle)

    # Summary chips: label over a big count, each on a soft colour.
    ws.set_row(5, 20); ws.set_row(6, 36)
    chips = [("B", "C", "Done", '=COUNTIF(D12:D23,"Done")', "#E3F1DC", "#2F6B2F"),
             ("D", "E", "In progress", '=COUNTIF(D12:D23,"In progress")', "#FFF1D1", "#8A5A00"),
             ("F", "G", "Blocked", '=COUNTIF(D12:D23,"Blocked")', "#FBE0DC", "#A33224"),
             ("H", "I", "Budget used", "=SUM(H12:H23)/SUM(G12:G23)", "#E6EEF6", "#23507A")]
    for a, b, label, formula, bg, fg in chips:
        ws.merge_range(f"{a}6:{b}6", label, f(bg_color=bg, font_color=fg, font_size=11, bold=True, indent=1))
        num = "0%" if "Budget" in label else "0"
        ws.merge_range(f"{a}7:{b}7", "", f(bg_color=bg, font_color=fg, font_size=22, bold=True, indent=1, align="left", num_format=num))
        ws.write_formula(f"{a}7", formula, f(bg_color=bg, font_color=fg, font_size=22, bold=True, indent=1, align="left", num_format=num))
    ws.set_row(7, 10)

    # Table
    ws.set_row(9, 16)
    ws.write("B10", "Tasks", f(bold=True, font_size=15))
    hdr = f(bold=True, font_size=11, font_color=muted, bg_color=cream, bottom=1, bottom_color=line)
    hdr_r = f(bold=True, font_size=11, font_color=muted, bg_color=cream, bottom=1, bottom_color=line, align="right")
    ws.set_row(10, 26)
    for c, h in enumerate(["Task", "Owner", "Status", "Priority", "Due"]):
        ws.write(10, 1 + c, h, hdr)
    for c, h in enumerate(["Budget", "Spent", "Progress"]):
        ws.write(10, 6 + c, h, hdr_r)
    for i, (task, owner, status, prio, due, budget, spent, prog) in enumerate(TASKS):
        r = 11 + i
        ws.set_row(r, 28)
        base = {"bg_color": stripe} if i % 2 else {}
        edge = {"bottom": 1, "bottom_color": line}
        ws.write(r, 1, task, f(**base, **edge, bold=True))
        ws.write(r, 2, owner, f(**base, **edge, font_color=muted))
        ws.write(r, 3, status, f(**base, **edge, align="center", bold=True, font_size=11))
        ws.write(r, 4, prio, f(**base, **edge, bold=True, font_size=11))
        ws.write(r, 5, due, f(**base, **edge, font_color=muted))
        ws.write_number(r, 6, budget, f(**base, **edge, num_format="$#,##0"))
        ws.write_number(r, 7, spent, f(**base, **edge, num_format="$#,##0"))
        ws.write_number(r, 8, prog, f(**base, **edge, num_format="0%"))
    body = "12:23"
    # Status pills
    for text, bg, fg in [("Done", "#DCEFD3", "#2F6B2F"), ("In progress", "#FFEBC2", "#8A5A00"),
                         ("Blocked", "#F8D5CF", "#A33224"), ("Not started", "#ECECE8", "#5F665F")]:
        ws.conditional_format(f"D{body.split(':')[0]}:D{body.split(':')[1]}", {"type": "cell", "criteria": "==", "value": f'"{text}"',
                              "format": wb.add_format({"bg_color": bg, "font_color": fg})})
    # Priority colour
    for text, fg in [("High", "#C0392B"), ("Medium", "#C27A00"), ("Low", "#3B7F3B")]:
        ws.conditional_format("E12:E23", {"type": "cell", "criteria": "==", "value": f'"{text}"', "format": wb.add_format({"font_color": fg})})
    # Over budget in red
    ws.conditional_format("H12:H23", {"type": "formula", "criteria": "=H12>G12", "format": wb.add_format({"font_color": "#C0392B", "bold": True})})
    # Progress bars
    ws.conditional_format("I12:I23", {"type": "data_bar", "bar_color": "#7FBF6A", "bar_solid": True, "bar_no_border": True, "min_type": "num", "min_value": 0, "max_type": "num", "max_value": 1})
    # Totals
    t = 24
    ws.set_row(t, 30)
    tot = f(bold=True, top=2, top_color=ink)
    ws.write(t, 1, "Total", tot)
    for c in range(2, 6):
        ws.write_blank(t, c, None, tot)
    ws.write_formula(t, 6, "=SUM(G12:G23)", f(bold=True, top=2, top_color=ink, num_format="$#,##0"))
    ws.write_formula(t, 7, "=SUM(H12:H23)", f(bold=True, top=2, top_color=ink, num_format="$#,##0"))
    ws.write_formula(t, 8, "=AVERAGE(I12:I23)", f(bold=True, top=2, top_color=ink, num_format="0%"))
    ws.activate()

    team = wb.add_worksheet("Team")
    team.set_column("A:A", 2.5); team.set_column("B:D", 18)
    team.write_row(1, 1, ["Name", "Role", "Tasks"], f(bold=True, font_color=muted, bg_color=cream))
    for i, (n, role) in enumerate([("Maya", "Owner"), ("Leo", "Head barista"), ("Priya", "Operations"), ("Sam", "Digital")]):
        team.write(2 + i, 1, n, f(bold=True)); team.write(2 + i, 2, role, f(font_color=muted))
        team.write_formula(2 + i, 3, f"=COUNTIF('Launch plan'!C12:C23,B{3 + i})", f())
    wb.add_worksheet("Notes").write("B2", "Demo data for screenshots. Every name and number is made up.", f(font_color=muted))
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
    overview_workbook(os.path.join(OUT, "Q3 Overview.xlsx"))
    launch_board(os.path.join(OUT, "Launch plan.xlsx"))
    contacts_csv(os.path.join(OUT, "contacts.csv"))
    # Short names for the tabs close-up (long names get truncated with four tabs).
    tabs = os.path.join(OUT, "tabs")
    os.makedirs(tabs, exist_ok=True)
    overview_workbook(os.path.join(tabs, "Overview.xlsx"))
    launch_board(os.path.join(tabs, "Launch plan.xlsx"))
    sales_workbook(os.path.join(tabs, "Sales.xlsx"))
    contacts_csv(os.path.join(tabs, "Contacts.csv"))
    import shutil
    shutil.copy(os.path.join(HERE, "..", "..", "testdata", "files", "formulas.xlsx"), os.path.join(tabs, "Budget.xlsx"))
    print("wrote", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
