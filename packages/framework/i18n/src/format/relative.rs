//! Relative-time formatting (`"2 hours ago"`, `"in 3 days"`, `"vor 2 Stunden"`, ...),
//! reusing CLDR cardinal plural rules and locale integer formatting.

#[cfg(feature = "builtin-fallback")]
use crate::format::number::builtin_format_integer;
#[cfg(feature = "builtin-fallback")]
use crate::locale::LocaleId;
#[cfg(feature = "builtin-fallback")]
use crate::plural::{PluralCategory, PluralKind, PluralOperands, builtin_plural_category};

/// Unit for relative-time formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelativeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl RelativeUnit {
    /// Returns the `Intl.RelativeTimeFormat` unit identifier (`"second"`, `"hour"`, `"day"`, ...).
    pub const fn as_intl_str(self) -> &'static str {
        match self {
            Self::Second => "second",
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }

    #[cfg(feature = "builtin-fallback")]
    const fn index(self) -> usize {
        match self {
            Self::Second => 0,
            Self::Minute => 1,
            Self::Hour => 2,
            Self::Day => 3,
            Self::Week => 4,
            Self::Month => 5,
            Self::Year => 6,
        }
    }
}

/// Formats a relative time offset using CLDR pluralization and locale integer
/// formatting (`value < 0` = past, `value >= 0` = future).
#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_relative_time(
    locale: &LocaleId,
    value: i64,
    unit: RelativeUnit,
) -> String {
    let past = value < 0;
    let abs_u64 = value.unsigned_abs();
    let op = PluralOperands::from_u64(abs_u64);
    let cat = builtin_plural_category(locale, PluralKind::Cardinal, op);
    let num = if let Ok(v) = i64::try_from(abs_u64) {
        builtin_format_integer(locale, v)
    } else {
        abs_u64.to_string()
    };

    match locale.language() {
        "de" => {
            // German dative plural for `vor`/`in`: Sekunden, Minuten, Stunden, Tagen, Wochen, Monaten, Jahren
            const ONE: [&str; 7] = ["Sekunde", "Minute", "Stunde", "Tag", "Woche", "Monat", "Jahr"];
            const MANY: [&str; 7] = ["Sekunden", "Minuten", "Stunden", "Tagen", "Wochen", "Monaten", "Jahren"];
            let u = if cat == PluralCategory::One { ONE[unit.index()] } else { MANY[unit.index()] };
            if past { format!("vor {num} {u}") } else { format!("in {num} {u}") }
        }
        "fr" => {
            const ONE: [&str; 7] = ["seconde", "minute", "heure", "jour", "semaine", "mois", "an"];
            const MANY: [&str; 7] = ["secondes", "minutes", "heures", "jours", "semaines", "mois", "ans"];
            let u = if cat == PluralCategory::One { ONE[unit.index()] } else { MANY[unit.index()] };
            if past { format!("il y a {num} {u}") } else { format!("dans {num} {u}") }
        }
        "es" => {
            const ONE: [&str; 7] = ["segundo", "minuto", "hora", "día", "semana", "mes", "año"];
            const MANY: [&str; 7] = ["segundos", "minutos", "horas", "días", "semanas", "meses", "años"];
            let u = if cat == PluralCategory::One { ONE[unit.index()] } else { MANY[unit.index()] };
            if past { format!("hace {num} {u}") } else { format!("dentro de {num} {u}") }
        }
        "it" => {
            const ONE: [&str; 7] = ["secondo", "minuto", "ora", "giorno", "settimana", "mese", "anno"];
            const MANY: [&str; 7] = ["secondi", "minuti", "ore", "giorni", "settimane", "mesi", "anni"];
            let u = if cat == PluralCategory::One { ONE[unit.index()] } else { MANY[unit.index()] };
            if past { format!("{num} {u} fa") } else { format!("tra {num} {u}") }
        }
        "pt" => {
            const ONE: [&str; 7] = ["segundo", "minuto", "hora", "dia", "semana", "mês", "ano"];
            const MANY: [&str; 7] = ["segundos", "minutos", "horas", "dias", "semanas", "meses", "anos"];
            let u = if cat == PluralCategory::One { ONE[unit.index()] } else { MANY[unit.index()] };
            if past { format!("há {num} {u}") } else { format!("em {num} {u}") }
        }
        "nl" => {
            const ONE: [&str; 7] = ["seconde", "minuut", "uur", "dag", "week", "maand", "jaar"];
            const MANY: [&str; 7] = ["seconden", "minuten", "uur", "dagen", "weken", "maanden", "jaar"];
            let u = if cat == PluralCategory::One { ONE[unit.index()] } else { MANY[unit.index()] };
            if past { format!("{num} {u} geleden") } else { format!("over {num} {u}") }
        }
        "sv" => {
            const ONE: [&str; 7] = ["sekund", "minut", "timme", "dag", "vecka", "månad", "år"];
            const MANY: [&str; 7] = ["sekunder", "minuter", "timmar", "dagar", "veckor", "månader", "år"];
            let u = if cat == PluralCategory::One { ONE[unit.index()] } else { MANY[unit.index()] };
            if past { format!("för {num} {u} sedan") } else { format!("om {num} {u}") }
        }
        "pl" => {
            const ONE: [&str; 7] = ["sekundę", "minutę", "godzinę", "dzień", "tydzień", "miesiąc", "rok"];
            const FEW: [&str; 7] = ["sekundy", "minuty", "godziny", "dni", "tygodnie", "miesiące", "lata"];
            const MANY: [&str; 7] = ["sekund", "minut", "godzin", "dni", "tygodni", "miesięcy", "lat"];
            let u = match cat {
                PluralCategory::One => ONE[unit.index()],
                PluralCategory::Few => FEW[unit.index()],
                _ => MANY[unit.index()],
            };
            if past { format!("{num} {u} temu") } else { format!("za {num} {u}") }
        }
        "ru" => {
            const ONE: [&str; 7] = ["секунду", "минуту", "час", "день", "неделю", "месяц", "год"];
            const FEW: [&str; 7] = ["секунды", "минуты", "часа", "дня", "недели", "месяца", "года"];
            const MANY: [&str; 7] = ["секунд", "минут", "часов", "дней", "недель", "месяцев", "лет"];
            let u = match cat {
                PluralCategory::One => ONE[unit.index()],
                PluralCategory::Few => FEW[unit.index()],
                _ => MANY[unit.index()],
            };
            if past { format!("{num} {u} назад") } else { format!("через {num} {u}") }
        }
        "ar" => {
            const ONE: [&str; 7] = ["ثانية", "دقيقة", "ساعة", "يوم", "أسبوع", "شهر", "سنة"];
            const TWO: [&str; 7] = ["ثانيتين", "دقيقتين", "ساعتين", "يومين", "أسبوعين", "شهرين", "سنتين"];
            const FEW: [&str; 7] = ["ثوانٍ", "دقائق", "ساعات", "أيام", "أسابيع", "أشهر", "سنوات"];
            let prefix = if past { "قبل" } else { "خلال" };
            match cat {
                PluralCategory::One => format!("{prefix} {}", ONE[unit.index()]),
                PluralCategory::Two => format!("{prefix} {}", TWO[unit.index()]),
                PluralCategory::Few => format!("{prefix} {num} {}", FEW[unit.index()]),
                _ => format!("{prefix} {num} {}", ONE[unit.index()]),
            }
        }
        "ja" => {
            const UNITS: [&str; 7] = ["秒", "分", "時間", "日", "週間", "か月", "年"];
            let u = UNITS[unit.index()];
            if past { format!("{num}{u}前") } else { format!("{num}{u}後") }
        }
        "zh" => {
            const UNITS: [&str; 7] = ["秒钟", "分钟", "小时", "天", "周", "个月", "年"];
            let u = UNITS[unit.index()];
            if past { format!("{num}{u}前") } else { format!("{num}{u}后") }
        }
        "ko" => {
            const UNITS: [&str; 7] = ["초", "분", "시간", "일", "주", "개월", "년"];
            let u = UNITS[unit.index()];
            if past { format!("{num}{u} 전") } else { format!("{num}{u} 후") }
        }
        "tr" => {
            const UNITS: [&str; 7] = ["saniye", "dakika", "saat", "gün", "hafta", "ay", "yıl"];
            let u = UNITS[unit.index()];
            if past { format!("{num} {u} önce") } else { format!("{num} {u} sonra") }
        }
        "vi" => {
            const UNITS: [&str; 7] = ["giây", "phút", "giờ", "ngày", "tuần", "tháng", "năm"];
            let u = UNITS[unit.index()];
            if past { format!("{num} {u} trước") } else { format!("sau {num} {u} nữa") }
        }
        _ => {
            const ONE: [&str; 7] = ["second", "minute", "hour", "day", "week", "month", "year"];
            const MANY: [&str; 7] = ["seconds", "minutes", "hours", "days", "weeks", "months", "years"];
            let u = if cat == PluralCategory::One { ONE[unit.index()] } else { MANY[unit.index()] };
            if past { format!("{num} {u} ago") } else { format!("in {num} {u}") }
        }
    }
}

/// Human-friendly relative-time formatting (`numeric: "auto"`): emits `"now"`,
/// `"today"`, `"yesterday"`, `"tomorrow"` (localized) when applicable, falling
/// back to [`builtin_format_relative_time`].
#[cfg(feature = "builtin-fallback")]
pub fn format_relative_time_auto(locale: &LocaleId, value: i64, unit: RelativeUnit) -> String {
    let lang = locale.language();
    match (unit, value) {
        (RelativeUnit::Second, 0) => match lang {
            "de" => "jetzt".to_owned(),
            "fr" => "maintenant".to_owned(),
            "es" => "ahora".to_owned(),
            "it" => "ora".to_owned(),
            "pt" => "agora".to_owned(),
            "nl" => "nu".to_owned(),
            "pl" => "teraz".to_owned(),
            "ru" => "сейчас".to_owned(),
            "ja" => "今".to_owned(),
            "zh" => "现在".to_owned(),
            "ko" => "지금".to_owned(),
            "ar" => "الآن".to_owned(),
            _ => "now".to_owned(),
        },
        (RelativeUnit::Day, 0) => match lang {
            "de" => "heute".to_owned(),
            "fr" => "aujourd'hui".to_owned(),
            "es" => "hoy".to_owned(),
            "it" => "oggi".to_owned(),
            "pt" => "hoje".to_owned(),
            "nl" => "vandaag".to_owned(),
            "pl" => "dzisiaj".to_owned(),
            "ru" => "сегодня".to_owned(),
            "ja" => "今日".to_owned(),
            "zh" => "今天".to_owned(),
            "ko" => "오늘".to_owned(),
            "ar" => "اليوم".to_owned(),
            _ => "today".to_owned(),
        },
        (RelativeUnit::Day, -1) => match lang {
            "de" => "gestern".to_owned(),
            "fr" => "hier".to_owned(),
            "es" => "ayer".to_owned(),
            "it" => "ieri".to_owned(),
            "pt" => "ontem".to_owned(),
            "nl" => "gisteren".to_owned(),
            "pl" => "wczoraj".to_owned(),
            "ru" => "вчера".to_owned(),
            "ja" => "昨日".to_owned(),
            "zh" => "昨天".to_owned(),
            "ko" => "어제".to_owned(),
            "ar" => "أمس".to_owned(),
            _ => "yesterday".to_owned(),
        },
        (RelativeUnit::Day, 1) => match lang {
            "de" => "morgen".to_owned(),
            "fr" => "demain".to_owned(),
            "es" => "mañana".to_owned(),
            "it" => "domani".to_owned(),
            "pt" => "amanhã".to_owned(),
            "nl" => "morgen".to_owned(),
            "pl" => "jutro".to_owned(),
            "ru" => "завтра".to_owned(),
            "ja" => "明日".to_owned(),
            "zh" => "明天".to_owned(),
            "ko" => "내일".to_owned(),
            "ar" => "غدًا".to_owned(),
            _ => "tomorrow".to_owned(),
        },
        _ => builtin_format_relative_time(locale, value, unit),
    }
}
