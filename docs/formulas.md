# Formula support

The `waffle-calc` engine implements **195 function names** (aliases such as `STDEV`/`STDEV.S` count separately). This list is generated from `crates/waffle-calc/src/funcs.rs`.

A formula that uses anything not listed here, or a structured table reference (`Table1[Col]`) or a link to another workbook, keeps the value Excel saved in the file instead of being recalculated.

## Math (52)

`SUM`, `SUMPRODUCT`, `SUMIF`, `SUMIFS`, `SUMSQ`, `PRODUCT`, `ABS`, `ROUND`, `ROUNDUP`, `ROUNDDOWN`, `MROUND`, `INT`, `TRUNC`, `CEILING`, `CEILING.MATH`, `CEILING.PRECISE`, `ISO.CEILING`, `FLOOR`, `FLOOR.MATH`, `FLOOR.PRECISE`, `MOD`, `POWER`, `SQRT`, `EXP`, `LN`, `LOG`, `LOG10`, `PI`, `SIGN`, `RAND`, `RANDBETWEEN`, `QUOTIENT`, `GCD`, `LCM`, `SUBTOTAL`, `AGGREGATE`, `EVEN`, `ODD`, `FACT`, `COMBIN`, `DEGREES`, `RADIANS`, `SIN`, `COS`, `TAN`, `ASIN`, `ACOS`, `ATAN`, `ATAN2`, `SINH`, `COSH`, `TANH`

## Stats (36)

`AVERAGE`, `AVERAGEA`, `AVERAGEIF`, `AVERAGEIFS`, `COUNT`, `COUNTA`, `COUNTBLANK`, `COUNTIF`, `COUNTIFS`, `MAX`, `MAXA`, `MIN`, `MINA`, `MAXIFS`, `MINIFS`, `MEDIAN`, `MODE`, `MODE.SNGL`, `LARGE`, `SMALL`, `RANK`, `RANK.EQ`, `STDEV`, `STDEV.S`, `STDEVP`, `STDEV.P`, `VAR`, `VAR.S`, `VARP`, `VAR.P`, `PERCENTILE`, `PERCENTILE.INC`, `PERCENTILE.EXC`, `QUARTILE`, `QUARTILE.INC`, `QUARTILE.EXC`

## Logic (12)

`IF`, `IFS`, `IFERROR`, `IFNA`, `AND`, `OR`, `XOR`, `NOT`, `TRUE`, `FALSE`, `SWITCH`, `CHOOSE`

## Lookup (15)

`VLOOKUP`, `HLOOKUP`, `XLOOKUP`, `XMATCH`, `LOOKUP`, `INDEX`, `MATCH`, `OFFSET`, `INDIRECT`, `ROW`, `ROWS`, `COLUMN`, `COLUMNS`, `ADDRESS`, `TRANSPOSE`

## Text (29)

`CONCATENATE`, `CONCAT`, `TEXTJOIN`, `LEFT`, `RIGHT`, `MID`, `LEN`, `LOWER`, `UPPER`, `PROPER`, `TRIM`, `CLEAN`, `SUBSTITUTE`, `REPLACE`, `FIND`, `SEARCH`, `EXACT`, `REPT`, `VALUE`, `NUMBERVALUE`, `TEXT`, `CHAR`, `CODE`, `UNICHAR`, `UNICODE`, `T`, `N`, `DOLLAR`, `FIXED`

## Date (24)

`DATE`, `DATEVALUE`, `TIME`, `TIMEVALUE`, `TODAY`, `NOW`, `YEAR`, `MONTH`, `DAY`, `HOUR`, `MINUTE`, `SECOND`, `WEEKDAY`, `WEEKNUM`, `ISOWEEKNUM`, `EDATE`, `EOMONTH`, `DATEDIF`, `DAYS`, `NETWORKDAYS`, `NETWORKDAYS.INTL`, `WORKDAY`, `WORKDAY.INTL`, `YEARFRAC`

## Info (14)

`ISBLANK`, `ISNUMBER`, `ISTEXT`, `ISNONTEXT`, `ISLOGICAL`, `ISERROR`, `ISERR`, `ISNA`, `ISEVEN`, `ISODD`, `ISREF`, `NA`, `ERROR.TYPE`, `TYPE`

## Financial (7)

`PMT`, `FV`, `PV`, `NPER`, `RATE`, `NPV`, `IRR`

## Dynamic arrays (5)

`UNIQUE`, `FILTER`, `SORT`, `SORTBY`, `SEQUENCE`

## Misc (1)

`SINGLE`

## Not yet supported

- Union and intersection reference operators, R1C1 references in `INDIRECT`, `LET`/`LAMBDA`, spill references (`A1#`).
- Array results spill only their top-left value into the cell.
- `SUBTOTAL`/`AGGREGATE` don't skip hidden rows.
- `TEXT` covers common number/date formats but not fractions or conditional sections.
