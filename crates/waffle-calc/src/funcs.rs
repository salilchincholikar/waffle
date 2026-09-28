//! Function identifiers: names are resolved once at compile time.

macro_rules! funcs {
    ($($v:ident => $($n:literal)|+;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub(crate) enum Func { $($v),* }
        /// Resolves an upper-case, prefix-stripped function name.
        pub(crate) fn lookup(name: &str) -> Option<Func> {
            match name { $($($n)|+ => Some(Func::$v),)* _ => None }
        }
        /// Every function name the engine implements.
        pub const SUPPORTED_FUNCTIONS: &[&str] = &[$($($n),+),*];
    };
}

funcs! {
    // math
    Sum => "SUM"; SumProduct => "SUMPRODUCT"; SumIf => "SUMIF"; SumIfs => "SUMIFS"; SumSq => "SUMSQ";
    Product => "PRODUCT"; Abs => "ABS"; Round => "ROUND"; RoundUp => "ROUNDUP"; RoundDown => "ROUNDDOWN";
    MRound => "MROUND"; Int => "INT"; Trunc => "TRUNC"; Ceiling => "CEILING";
    CeilingMath => "CEILING.MATH"; CeilingPrecise => "CEILING.PRECISE" | "ISO.CEILING";
    Floor => "FLOOR"; FloorMath => "FLOOR.MATH"; FloorPrecise => "FLOOR.PRECISE";
    Mod => "MOD"; Power => "POWER"; Sqrt => "SQRT"; Exp => "EXP"; Ln => "LN"; Log => "LOG"; Log10 => "LOG10";
    Pi => "PI"; Sign => "SIGN"; Rand => "RAND"; RandBetween => "RANDBETWEEN"; Quotient => "QUOTIENT";
    Gcd => "GCD"; Lcm => "LCM"; Subtotal => "SUBTOTAL"; Aggregate => "AGGREGATE"; Even => "EVEN"; Odd => "ODD";
    Fact => "FACT"; Combin => "COMBIN"; Degrees => "DEGREES"; Radians => "RADIANS";
    Sin => "SIN"; Cos => "COS"; Tan => "TAN"; Asin => "ASIN"; Acos => "ACOS"; Atan => "ATAN"; Atan2 => "ATAN2";
    Sinh => "SINH"; Cosh => "COSH"; Tanh => "TANH";
    // stats
    Average => "AVERAGE"; AverageA => "AVERAGEA"; AverageIf => "AVERAGEIF"; AverageIfs => "AVERAGEIFS";
    Count => "COUNT"; CountA => "COUNTA"; CountBlank => "COUNTBLANK"; CountIf => "COUNTIF"; CountIfs => "COUNTIFS";
    Max => "MAX"; MaxA => "MAXA"; Min => "MIN"; MinA => "MINA"; MaxIfs => "MAXIFS"; MinIfs => "MINIFS";
    Median => "MEDIAN"; Mode => "MODE" | "MODE.SNGL"; Large => "LARGE"; Small => "SMALL"; Rank => "RANK" | "RANK.EQ";
    StdevS => "STDEV" | "STDEV.S"; StdevP => "STDEVP" | "STDEV.P"; VarS => "VAR" | "VAR.S"; VarP => "VARP" | "VAR.P";
    PercentileInc => "PERCENTILE" | "PERCENTILE.INC"; PercentileExc => "PERCENTILE.EXC";
    QuartileInc => "QUARTILE" | "QUARTILE.INC"; QuartileExc => "QUARTILE.EXC";
    // logic
    If => "IF"; Ifs => "IFS"; IfError => "IFERROR"; IfNa => "IFNA"; And => "AND"; Or => "OR"; Xor => "XOR";
    Not => "NOT"; True => "TRUE"; False => "FALSE"; Switch => "SWITCH"; Choose => "CHOOSE";
    // lookup
    VLookup => "VLOOKUP"; HLookup => "HLOOKUP"; XLookup => "XLOOKUP"; XMatch => "XMATCH"; Lookup => "LOOKUP";
    Index => "INDEX"; Match => "MATCH"; Offset => "OFFSET"; Indirect => "INDIRECT"; Row => "ROW"; Rows => "ROWS";
    Column => "COLUMN"; Columns => "COLUMNS"; Address => "ADDRESS"; Transpose => "TRANSPOSE";
    // text
    Concatenate => "CONCATENATE"; Concat => "CONCAT"; TextJoin => "TEXTJOIN"; Left => "LEFT"; Right => "RIGHT";
    Mid => "MID"; Len => "LEN"; Lower => "LOWER"; Upper => "UPPER"; Proper => "PROPER"; Trim => "TRIM";
    Clean => "CLEAN"; Substitute => "SUBSTITUTE"; Replace => "REPLACE"; Find => "FIND"; Search => "SEARCH";
    Exact => "EXACT"; Rept => "REPT"; ValueFn => "VALUE"; NumberValue => "NUMBERVALUE"; Text => "TEXT";
    Char => "CHAR"; Code => "CODE"; Unichar => "UNICHAR"; Unicode => "UNICODE"; T => "T"; N => "N";
    Dollar => "DOLLAR"; Fixed => "FIXED";
    // date
    Date => "DATE"; DateValue => "DATEVALUE"; Time => "TIME"; TimeValue => "TIMEVALUE"; Today => "TODAY";
    Now => "NOW"; Year => "YEAR"; Month => "MONTH"; Day => "DAY"; Hour => "HOUR"; Minute => "MINUTE";
    Second => "SECOND"; Weekday => "WEEKDAY"; WeekNum => "WEEKNUM"; IsoWeekNum => "ISOWEEKNUM";
    EDate => "EDATE"; EOMonth => "EOMONTH"; DateDif => "DATEDIF"; Days => "DAYS";
    NetworkDays => "NETWORKDAYS"; NetworkDaysIntl => "NETWORKDAYS.INTL"; Workday => "WORKDAY";
    WorkdayIntl => "WORKDAY.INTL"; YearFrac => "YEARFRAC";
    // info
    IsBlank => "ISBLANK"; IsNumber => "ISNUMBER"; IsText => "ISTEXT"; IsNonText => "ISNONTEXT";
    IsLogical => "ISLOGICAL"; IsError => "ISERROR"; IsErr => "ISERR"; IsNa => "ISNA"; IsEven => "ISEVEN";
    IsOdd => "ISODD"; IsRef => "ISREF"; Na => "NA"; ErrorType => "ERROR.TYPE"; Type => "TYPE";
    // financial
    Pmt => "PMT"; Fv => "FV"; Pv => "PV"; NPer => "NPER"; Rate => "RATE"; Npv => "NPV"; Irr => "IRR";
    // dynamic arrays
    Unique => "UNIQUE"; Filter => "FILTER"; Sort => "SORT"; SortBy => "SORTBY"; Sequence => "SEQUENCE";
    // misc
    Single => "SINGLE";
}

impl Func {
    pub(crate) fn is_volatile(self) -> bool {
        matches!(self, Func::Today | Func::Now | Func::Rand | Func::RandBetween | Func::Offset | Func::Indirect)
    }

    /// Functions whose arguments are all scalars: evaluated eagerly and
    /// lifted element-wise over array arguments.
    pub(crate) fn is_scalar(self) -> bool {
        use Func::*;
        !matches!(
            self,
            Sum | SumProduct
                | SumIf
                | SumIfs
                | SumSq
                | Product
                | Rand
                | Gcd
                | Lcm
                | Subtotal
                | Aggregate
                | Average
                | AverageA
                | AverageIf
                | AverageIfs
                | Count
                | CountA
                | CountBlank
                | CountIf
                | CountIfs
                | Max
                | MaxA
                | Min
                | MinA
                | MaxIfs
                | MinIfs
                | Median
                | Mode
                | Large
                | Small
                | Rank
                | StdevS
                | StdevP
                | VarS
                | VarP
                | PercentileInc
                | PercentileExc
                | QuartileInc
                | QuartileExc
                | If
                | Ifs
                | IfError
                | IfNa
                | And
                | Or
                | Xor
                | Switch
                | Choose
                | VLookup
                | HLookup
                | XLookup
                | XMatch
                | Lookup
                | Index
                | Match
                | Offset
                | Indirect
                | Row
                | Rows
                | Column
                | Columns
                | Transpose
                | Concat
                | TextJoin
                | NetworkDays
                | NetworkDaysIntl
                | Workday
                | WorkdayIntl
                | IsRef
                | Type
                | Npv
                | Irr
                | Unique
                | Filter
                | Sort
                | SortBy
                | Sequence
                | Single
        )
    }
}
