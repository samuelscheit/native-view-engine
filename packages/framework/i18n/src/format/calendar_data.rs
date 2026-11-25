//! Localized CLDR month and weekday tables compiled only when
//! `feature = "builtin-fallback"` is enabled.

pub(crate) struct MonthNames {
    pub wide: [&'static str; 12],
    pub abbr: [&'static str; 12],
    pub narrow: [&'static str; 12],
}

pub(crate) struct WeekdayNames {
    /// Indexed Sunday = 0 .. Saturday = 6.
    pub wide: [&'static str; 7],
    pub abbr: [&'static str; 7],
    pub narrow: [&'static str; 7],
}

pub(crate) fn months_for(lang: &str) -> &'static MonthNames {
    match lang {
        "de" => &MONTHS_DE,
        "fr" => &MONTHS_FR,
        "es" => &MONTHS_ES,
        "it" => &MONTHS_IT,
        "pt" => &MONTHS_PT,
        "nl" => &MONTHS_NL,
        "sv" => &MONTHS_SV,
        "pl" => &MONTHS_PL,
        "ru" => &MONTHS_RU,
        "uk" => &MONTHS_UK,
        "cs" => &MONTHS_CS,
        "sk" => &MONTHS_SK,
        "ar" => &MONTHS_AR,
        "ja" => &MONTHS_JA,
        "zh" => &MONTHS_ZH,
        "ko" => &MONTHS_KO,
        "tr" => &MONTHS_TR,
        "vi" => &MONTHS_VI,
        "cy" => &MONTHS_CY,
        _ => &MONTHS_EN,
    }
}

pub(crate) fn weekdays_for(lang: &str) -> &'static WeekdayNames {
    match lang {
        "de" => &WEEKDAYS_DE,
        "fr" => &WEEKDAYS_FR,
        "es" => &WEEKDAYS_ES,
        "it" => &WEEKDAYS_IT,
        "pt" => &WEEKDAYS_PT,
        "nl" => &WEEKDAYS_NL,
        "sv" => &WEEKDAYS_SV,
        "pl" => &WEEKDAYS_PL,
        "ru" => &WEEKDAYS_RU,
        "uk" => &WEEKDAYS_UK,
        "cs" => &WEEKDAYS_CS,
        "sk" => &WEEKDAYS_SK,
        "ar" => &WEEKDAYS_AR,
        "ja" => &WEEKDAYS_JA,
        "zh" => &WEEKDAYS_ZH,
        "ko" => &WEEKDAYS_KO,
        "tr" => &WEEKDAYS_TR,
        "vi" => &WEEKDAYS_VI,
        "cy" => &WEEKDAYS_CY,
        _ => &WEEKDAYS_EN,
    }
}

static MONTHS_EN: MonthNames = MonthNames {
    wide: [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December",
    ],
    abbr: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun",
        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    narrow: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"],
};

static MONTHS_DE: MonthNames = MonthNames {
    wide: [
        "Januar", "Februar", "März", "April", "Mai", "Juni",
        "Juli", "August", "September", "Oktober", "November", "Dezember",
    ],
    abbr: [
        "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni",
        "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez.",
    ],
    narrow: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"],
};

static MONTHS_FR: MonthNames = MonthNames {
    wide: [
        "janvier", "février", "mars", "avril", "mai", "juin",
        "juillet", "août", "septembre", "octobre", "novembre", "décembre",
    ],
    abbr: [
        "janv.", "févr.", "mars", "avr.", "mai", "juin",
        "juil.", "août", "sept.", "oct.", "nov.", "déc.",
    ],
    narrow: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"],
};

static MONTHS_ES: MonthNames = MonthNames {
    wide: [
        "enero", "febrero", "marzo", "abril", "mayo", "junio",
        "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
    ],
    abbr: [
        "ene", "feb", "mar", "abr", "may", "jun",
        "jul", "ago", "sept", "oct", "nov", "dic",
    ],
    narrow: ["E", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"],
};

static MONTHS_IT: MonthNames = MonthNames {
    wide: [
        "gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno",
        "luglio", "agosto", "settembre", "ottobre", "novembre", "dicembre",
    ],
    abbr: [
        "gen", "feb", "mar", "apr", "mag", "giu",
        "lug", "ago", "set", "ott", "nov", "dic",
    ],
    narrow: ["G", "F", "M", "A", "M", "G", "L", "A", "S", "O", "N", "D"],
};

static MONTHS_PT: MonthNames = MonthNames {
    wide: [
        "janeiro", "fevereiro", "março", "abril", "maio", "junho",
        "julho", "agosto", "setembro", "outubro", "novembro", "dezembro",
    ],
    abbr: [
        "jan.", "fev.", "mar.", "abr.", "mai.", "jun.",
        "jul.", "ago.", "set.", "out.", "nov.", "dez.",
    ],
    narrow: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"],
};

static MONTHS_NL: MonthNames = MonthNames {
    wide: [
        "januari", "februari", "maart", "april", "mei", "juni",
        "juli", "augustus", "september", "oktober", "november", "december",
    ],
    abbr: [
        "jan", "feb", "mrt", "apr", "mei", "jun",
        "jul", "aug", "sep", "okt", "nov", "dec",
    ],
    narrow: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"],
};

static MONTHS_SV: MonthNames = MonthNames {
    wide: [
        "januari", "februari", "mars", "april", "maj", "juni",
        "juli", "augusti", "september", "oktober", "november", "december",
    ],
    abbr: [
        "jan.", "feb.", "mars", "apr.", "maj", "juni",
        "juli", "aug.", "sep.", "okt.", "nov.", "dec.",
    ],
    narrow: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"],
};

static MONTHS_PL: MonthNames = MonthNames {
    wide: [
        "stycznia", "lutego", "marca", "kwietnia", "maja", "czerwca",
        "lipca", "sierpnia", "września", "października", "listopada", "grudnia",
    ],
    abbr: [
        "sty", "lut", "mar", "kwi", "maj", "cze",
        "lip", "sie", "wrz", "paź", "lis", "gru",
    ],
    narrow: ["S", "L", "M", "K", "M", "C", "L", "S", "W", "P", "L", "G"],
};

static MONTHS_RU: MonthNames = MonthNames {
    wide: [
        "января", "февраля", "марта", "апреля", "мая", "июня",
        "июля", "августа", "сентября", "октября", "ноября", "декабря",
    ],
    abbr: [
        "янв.", "февр.", "мар.", "апр.", "мая", "июн.",
        "июл.", "авг.", "сент.", "окт.", "нояб.", "дек.",
    ],
    narrow: ["Я", "Ф", "М", "А", "М", "И", "И", "А", "С", "О", "Н", "Д"],
};

static MONTHS_UK: MonthNames = MonthNames {
    wide: [
        "січня", "лютого", "березня", "квітня", "травня", "червня",
        "липня", "серпня", "вересня", "жовтня", "листопада", "грудня",
    ],
    abbr: [
        "січ.", "лют.", "бер.", "квіт.", "трав.", "черв.",
        "лип.", "серп.", "вер.", "жовт.", "лист.", "груд.",
    ],
    narrow: ["С", "Л", "Б", "К", "Т", "Ч", "Л", "С", "В", "Ж", "Л", "Г"],
};

static MONTHS_CS: MonthNames = MonthNames {
    wide: [
        "ledna", "února", "března", "dubna", "května", "června",
        "července", "srpna", "září", "října", "listopadu", "prosince",
    ],
    abbr: [
        "led", "úno", "bře", "dub", "kvě", "čvn",
        "čvc", "srp", "zář", "říj", "lis", "pro",
    ],
    narrow: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"],
};

static MONTHS_SK: MonthNames = MonthNames {
    wide: [
        "januára", "februára", "marca", "apríla", "mája", "júna",
        "júla", "augusta", "septembra", "októbra", "novembra", "decembra",
    ],
    abbr: [
        "jan", "feb", "mar", "apr", "máj", "jún",
        "júl", "aug", "sep", "okt", "nov", "dec",
    ],
    narrow: ["j", "f", "m", "a", "m", "j", "j", "a", "s", "o", "n", "d"],
};

static MONTHS_AR: MonthNames = MonthNames {
    wide: [
        "يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو",
        "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر",
    ],
    abbr: [
        "يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو",
        "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر",
    ],
    narrow: ["ي", "ف", "م", "أ", "و", "ن", "ل", "غ", "س", "ك", "ب", "د"],
};

static MONTHS_JA: MonthNames = MonthNames {
    wide: [
        "1月", "2月", "3月", "4月", "5月", "6月",
        "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    abbr: [
        "1月", "2月", "3月", "4月", "5月", "6月",
        "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    narrow: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"],
};

static MONTHS_ZH: MonthNames = MonthNames {
    wide: [
        "一月", "二月", "三月", "四月", "五月", "六月",
        "七月", "八月", "九月", "十月", "十一月", "十二月",
    ],
    abbr: [
        "1月", "2月", "3月", "4月", "5月", "6月",
        "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    narrow: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"],
};

static MONTHS_KO: MonthNames = MonthNames {
    wide: [
        "1월", "2월", "3월", "4월", "5월", "6월",
        "7월", "8월", "9월", "10월", "11월", "12월",
    ],
    abbr: [
        "1월", "2월", "3월", "4월", "5월", "6월",
        "7월", "8월", "9월", "10월", "11월", "12월",
    ],
    narrow: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"],
};

static MONTHS_TR: MonthNames = MonthNames {
    wide: [
        "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran",
        "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık",
    ],
    abbr: [
        "Oca", "Şub", "Mar", "Nis", "May", "Haz",
        "Tem", "Ağu", "Eyl", "Eki", "Kas", "Ara",
    ],
    narrow: ["O", "Ş", "M", "N", "M", "H", "T", "A", "E", "E", "K", "A"],
};

static MONTHS_VI: MonthNames = MonthNames {
    wide: [
        "tháng 1", "tháng 2", "tháng 3", "tháng 4", "tháng 5", "tháng 6",
        "tháng 7", "tháng 8", "tháng 9", "tháng 10", "tháng 11", "tháng 12",
    ],
    abbr: [
        "thg 1", "thg 2", "thg 3", "thg 4", "thg 5", "thg 6",
        "thg 7", "thg 8", "thg 9", "thg 10", "thg 11", "thg 12",
    ],
    narrow: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"],
};

static MONTHS_CY: MonthNames = MonthNames {
    wide: [
        "Ionawr", "Chwefror", "Mawrth", "Ebrill", "Mai", "Mehefin",
        "Gorffennaf", "Awst", "Medi", "Hydref", "Tachwedd", "Rhagfyr",
    ],
    abbr: [
        "Ion", "Chwef", "Maw", "Ebr", "Mai", "Meh",
        "Gorff", "Awst", "Medi", "Hyd", "Tach", "Rhag",
    ],
    narrow: ["I", "Ch", "M", "E", "M", "M", "G", "A", "M", "H", "T", "Rh"],
};

static WEEKDAYS_EN: WeekdayNames = WeekdayNames {
    wide: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
    abbr: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    narrow: ["S", "M", "T", "W", "T", "F", "S"],
};

static WEEKDAYS_DE: WeekdayNames = WeekdayNames {
    wide: ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"],
    abbr: ["So.", "Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa."],
    narrow: ["S", "M", "D", "M", "D", "F", "S"],
};

static WEEKDAYS_FR: WeekdayNames = WeekdayNames {
    wide: ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"],
    abbr: ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."],
    narrow: ["D", "L", "M", "M", "J", "V", "S"],
};

static WEEKDAYS_ES: WeekdayNames = WeekdayNames {
    wide: ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"],
    abbr: ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"],
    narrow: ["D", "L", "M", "X", "J", "V", "S"],
};

static WEEKDAYS_IT: WeekdayNames = WeekdayNames {
    wide: ["domenica", "lunedì", "martedì", "mercoledì", "giovedì", "venerdì", "sabato"],
    abbr: ["dom", "lun", "mar", "mer", "gio", "ven", "sab"],
    narrow: ["D", "L", "M", "M", "G", "V", "S"],
};

static WEEKDAYS_PT: WeekdayNames = WeekdayNames {
    wide: [
        "domingo", "segunda-feira", "terça-feira", "quarta-feira",
        "quinta-feira", "sexta-feira", "sábado",
    ],
    abbr: ["dom.", "seg.", "ter.", "qua.", "qui.", "sex.", "sáb."],
    narrow: ["D", "S", "T", "Q", "Q", "S", "S"],
};

static WEEKDAYS_NL: WeekdayNames = WeekdayNames {
    wide: ["zondag", "maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag"],
    abbr: ["zo", "ma", "di", "wo", "do", "vr", "za"],
    narrow: ["Z", "M", "D", "W", "D", "V", "Z"],
};

static WEEKDAYS_SV: WeekdayNames = WeekdayNames {
    wide: ["söndag", "måndag", "tisdag", "onsdag", "torsdag", "fredag", "lördag"],
    abbr: ["sön", "mån", "tis", "ons", "tors", "fre", "lör"],
    narrow: ["S", "M", "T", "O", "T", "F", "L"],
};

static WEEKDAYS_PL: WeekdayNames = WeekdayNames {
    wide: ["niedziela", "poniedziałek", "wtorek", "środa", "czwartek", "piątek", "sobota"],
    abbr: ["niedz.", "pon.", "wt.", "śr.", "czw.", "pt.", "sob."],
    narrow: ["N", "P", "W", "Ś", "C", "P", "S"],
};

static WEEKDAYS_RU: WeekdayNames = WeekdayNames {
    wide: ["воскресенье", "понедельник", "вторник", "среда", "четверг", "пятница", "суббота"],
    abbr: ["вс", "пн", "вт", "ср", "чт", "пт", "сб"],
    narrow: ["В", "П", "В", "С", "Ч", "П", "С"],
};

static WEEKDAYS_UK: WeekdayNames = WeekdayNames {
    wide: ["неділя", "понеділок", "вівторок", "середа", "четвер", "пʼятниця", "субота"],
    abbr: ["нд", "пн", "вт", "ср", "чт", "пт", "сб"],
    narrow: ["Н", "П", "В", "С", "Ч", "П", "С"],
};

static WEEKDAYS_CS: WeekdayNames = WeekdayNames {
    wide: ["neděle", "pondělí", "úterý", "středa", "čtvrtek", "pátek", "sobota"],
    abbr: ["ne", "po", "út", "st", "čt", "pá", "so"],
    narrow: ["N", "P", "Ú", "S", "Č", "P", "S"],
};

static WEEKDAYS_SK: WeekdayNames = WeekdayNames {
    wide: ["nedeľa", "pondelok", "utorok", "streda", "štvrtok", "piatok", "sobota"],
    abbr: ["ne", "po", "ut", "st", "št", "pi", "so"],
    narrow: ["n", "p", "u", "s", "š", "p", "s"],
};

static WEEKDAYS_AR: WeekdayNames = WeekdayNames {
    wide: ["الأحد", "الاثنين", "الثلاثاء", "الأربعاء", "الخميس", "الجمعة", "السبت"],
    abbr: ["الأحد", "الاثنين", "الثلاثاء", "الأربعاء", "الخميس", "الجمعة", "السبت"],
    narrow: ["ح", "ن", "ث", "ر", "خ", "ج", "س"],
};

static WEEKDAYS_JA: WeekdayNames = WeekdayNames {
    wide: ["日曜日", "月曜日", "火曜日", "水曜日", "木曜日", "金曜日", "土曜日"],
    abbr: ["日", "月", "火", "水", "木", "金", "土"],
    narrow: ["日", "月", "火", "水", "木", "金", "土"],
};

static WEEKDAYS_ZH: WeekdayNames = WeekdayNames {
    wide: ["星期日", "星期一", "星期二", "星期三", "星期四", "星期五", "星期六"],
    abbr: ["周日", "周一", "周二", "周三", "周四", "周五", "周六"],
    narrow: ["日", "一", "二", "三", "四", "五", "六"],
};

static WEEKDAYS_KO: WeekdayNames = WeekdayNames {
    wide: ["일요일", "월요일", "화요일", "수요일", "목요일", "금요일", "토요일"],
    abbr: ["일", "월", "화", "수", "목", "금", "토"],
    narrow: ["일", "월", "화", "수", "목", "금", "토"],
};

static WEEKDAYS_TR: WeekdayNames = WeekdayNames {
    wide: ["Pazar", "Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi"],
    abbr: ["Paz", "Pzt", "Sal", "Çar", "Per", "Cum", "Cmt"],
    narrow: ["P", "P", "S", "Ç", "P", "C", "C"],
};

static WEEKDAYS_VI: WeekdayNames = WeekdayNames {
    wide: ["Chủ Nhật", "Thứ Hai", "Thứ Ba", "Thứ Tư", "Thứ Năm", "Thứ Sáu", "Thứ Bảy"],
    abbr: ["CN", "Th 2", "Th 3", "Th 4", "Th 5", "Th 6", "Th 7"],
    narrow: ["CN", "T2", "T3", "T4", "T5", "T6", "T7"],
};

static WEEKDAYS_CY: WeekdayNames = WeekdayNames {
    wide: [
        "Dydd Sul", "Dydd Llun", "Dydd Mawrth", "Dydd Mercher",
        "Dydd Iau", "Dydd Gwener", "Dydd Sadwrn",
    ],
    abbr: ["Sul", "Llun", "Maw", "Mer", "Iau", "Gwen", "Sad"],
    narrow: ["S", "Ll", "M", "M", "I", "G", "S"],
};
