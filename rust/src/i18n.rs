//! Screen text in each language.
//!
//! Hardware labels (CPU, GPU, VRAM, Network, Fans, ...) and the AI block headings stay in
//! English in every language, like on most hardware monitors; everything a person reads as a
//! sentence is translated here. Dates use chrono's locale data. Right-to-left scripts and
//! scripts that need complex shaping (Arabic, Hebrew, Hindi, Thai) are not supported.
//!
//! Placeholders: `{}` in a string is replaced by the value; `{d}` `{h}` `{m}` are days, hours
//! and minutes.

use std::sync::OnceLock;

use chrono::Locale;

use crate::config;

/// Which installed Windows font to prefer for text (the bundled Noto Sans KR covers Korean,
/// kana, Han, Cyrillic and Greek; HarmonyOS Sans and Segoe UI fill the gaps).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextFont {
    Noto,
    YuGothic,   // Japanese glyph forms
    YaHei,      // Simplified Chinese
    JhengHei,   // Traditional Chinese
}

pub struct Strings {
    pub code: &'static str,
    pub locale: Locale,
    pub font: TextFont,
    /// date line under the clock (chrono format)
    pub date: &'static str,
    /// calendar heading (chrono format) and whether it is set in letter-spaced capitals
    pub month: &'static str,
    pub month_caps: bool,
    /// format the calendar heading with English month names
    pub month_en: bool,
    /// calendar heading month names, where the locale's %B is the genitive (Russian, Polish)
    pub month_names: Option<[&'static str; 12]>,
    /// calendar column heads, Sunday first
    pub weekdays: [&'static str; 7],
    pub loading: &'static str,
    pub feels: &'static str,
    pub humidity: &'static str,
    pub low: &'static str,
    pub high: &'static str,
    pub rain: &'static str,
    pub tomorrow: &'static str,
    pub tomorrow_caps: bool,
    pub pm25: &'static str,
    pub pm10: &'static str,
    /// air quality: good, moderate, bad, very bad, hazardous, watch, warning
    pub air: [&'static str; 7],
    /// weather (see `weather_index`)
    pub weather: [&'static str; 16],
    /// time until a limit resets: days+hours, hours+minutes, minutes
    pub left: [&'static str; 3],
    pub refilled: &'static str,
    pub rate_limited: &'static str,
    pub rate_limited_soon: &'static str,
    pub signed_out: &'static str,
    pub failed: &'static str,
    /// default names of the two board fan channels and the GPU fan
    pub fans: [&'static str; 3],
    /// subtitles of the three standby screens
    pub standby: [&'static str; 3],
}

/// Supported languages, in the order of `config.toml`'s comment.
pub const LANGUAGES: [&Strings; 16] = [&EN, &KO, &JA, &ZH_CN, &ZH_TW, &ES, &FR, &DE, &IT, &PT, &RU, &PL, &TR, &NL, &VI, &ID];

/// The language picked in the settings ("auto": the Windows locale).
pub fn get() -> &'static Strings {
    static S: OnceLock<&'static Strings> = OnceLock::new();
    S.get_or_init(|| {
        // TURZX_LANGUAGE overrides the settings (previews: `diag layout`)
        let want = std::env::var("TURZX_LANGUAGE").unwrap_or_else(|_| config::get().language.trim().to_string());
        let code = if want.is_empty() || want.eq_ignore_ascii_case("auto") { from_locale(&config::windows_locale()) } else { want };
        LANGUAGES.iter().copied().find(|s| s.code.eq_ignore_ascii_case(&code)).unwrap_or_else(|| {
            log::warn!("language {code:?} not supported; using English");
            &EN
        })
    })
}

/// Language code for a Windows locale name ("ko-KR" -> "ko", "zh-Hant-TW" -> "zh-TW").
fn from_locale(locale: &str) -> String {
    let lower = locale.to_ascii_lowercase();
    let lang = lower.split('-').next().unwrap_or("en");
    match lang {
        "zh" if ["-tw", "-hk", "-mo", "hant"].iter().any(|k| lower.contains(k)) => "zh-TW".into(),
        "zh" => "zh-CN".into(),
        "nb" | "nn" | "no" => "en".into(),
        other => other.into(),
    }
}

/// Index into `Strings::weather` for a WMO weather code.
pub fn weather_index(code: i64) -> Option<usize> {
    Some(match code {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        45 | 48 => 4,
        51 | 53 | 55 => 5,
        56 | 57 | 66 | 67 => 6,
        61 | 63 => 7,
        65 => 8,
        71 | 73 => 9,
        75 => 10,
        77 => 11,
        80 | 81 => 12,
        82 => 13,
        85 | 86 => 14,
        95 | 96 | 99 => 15,
        _ => return None,
    })
}

impl Strings {
    pub fn weather_label(&self, code: i64) -> &'static str {
        weather_index(code).map_or("-", |i| self.weather[i])
    }

    /// "2h 13m" style time until `secs` from now.
    pub fn time_left(&self, secs: i64) -> String {
        let s = secs.max(0);
        let (d, h, m) = (s / 86400, s % 86400 / 3600, s % 3600 / 60);
        let pat = if d > 0 { self.left[0] } else if h > 0 { self.left[1] } else { self.left[2] };
        pat.replace("{d}", &d.to_string()).replace("{h}", &h.to_string()).replace("{m}", &m.to_string())
    }
}

/// `pat` with its `{}` replaced by `v`.
pub fn fill(pat: &str, v: impl std::fmt::Display) -> String {
    pat.replacen("{}", &v.to_string(), 1)
}

pub static EN: Strings = Strings {
    code: "en",
    locale: Locale::en_US,
    font: TextFont::Noto,
    date: "%A, %B %-d, %Y",
    month: "%B",
    month_caps: true,
    month_en: true,
    month_names: None,
    weekdays: ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"],
    loading: "Loading weather…",
    feels: "Feels {}",
    humidity: "Humidity {}%",
    low: "Low {}",
    high: "High {}",
    rain: "Rain {}%",
    tomorrow: "Tomorrow",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Good", "Fair", "Poor", "Very poor", "Hazardous", "Alert", "Warning"],
    weather: [
        "Clear", "Mostly clear", "Partly cloudy", "Overcast", "Fog", "Drizzle", "Freezing rain", "Rain", "Heavy rain", "Snow",
        "Heavy snow", "Snow grains", "Showers", "Heavy showers", "Snow showers", "Thunderstorm",
    ],
    left: ["{d}d {h}h", "{h}h {m}m", "{m}m"],
    refilled: "Reset · checking",
    rate_limited: "Rate limited · retry in {}",
    rate_limited_soon: "Rate limited · retrying soon",
    signed_out: "Signed out · open Claude Code",
    failed: "Failed ({})",
    fans: ["Radiator", "Pump", "GPU"],
    standby: ["Good night · see you soon", "See you later!", "Taking a short break"],
};

pub static KO: Strings = Strings {
    code: "ko",
    locale: Locale::ko_KR,
    font: TextFont::Noto,
    date: "%Y. %m. %d   %A",
    month: "%B",
    month_caps: true,
    month_en: true,
    month_names: None,
    weekdays: ["일", "월", "화", "수", "목", "금", "토"],
    loading: "날씨 불러오는 중…",
    feels: "체감 {}",
    humidity: "습도 {}%",
    low: "최저 {}",
    high: "최고 {}",
    rain: "강수 {}%",
    tomorrow: "Tomorrow",
    tomorrow_caps: true,
    pm25: "초미세",
    pm10: "미세",
    air: ["좋음", "보통", "나쁨", "매우나쁨", "위험", "주의보", "경보"],
    weather: [
        "맑음", "대체로 맑음", "구름 조금", "흐림", "안개", "이슬비", "어는 비", "비", "강한 비", "눈", "많은 눈", "싸락눈", "소나기",
        "강한 소나기", "눈 소나기", "뇌우",
    ],
    left: ["{d}일 {h}시간", "{h}시간 {m}분", "{m}분"],
    refilled: "초기화됨 · 확인 중",
    rate_limited: "요청 제한 · {} 후 재시도",
    rate_limited_soon: "요청 제한 · 곧 재시도",
    signed_out: "로그인 만료 · Claude Code 실행 필요",
    failed: "조회 실패 ({})",
    fans: ["라디에이터", "펌프", "VGA"],
    standby: ["잘 자요 · 곧 다시 만나요", "다녀올게요!", "잠시 쉬는 중"],
};

pub static JA: Strings = Strings {
    code: "ja",
    locale: Locale::ja_JP,
    font: TextFont::YuGothic,
    date: "%Y年%-m月%-d日 %A",
    month: "%-m月",
    month_caps: false,
    month_en: false,
    month_names: None,
    weekdays: ["日", "月", "火", "水", "木", "金", "土"],
    loading: "天気を取得中…",
    feels: "体感 {}",
    humidity: "湿度 {}%",
    low: "最低 {}",
    high: "最高 {}",
    rain: "降水 {}%",
    tomorrow: "明日",
    tomorrow_caps: false,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["良好", "普通", "悪い", "非常に悪い", "危険", "注意報", "警報"],
    weather: [
        "快晴", "晴れ", "晴れ時々曇り", "曇り", "霧", "霧雨", "着氷性の雨", "雨", "強い雨", "雪", "大雪", "霧雪", "にわか雨",
        "強いにわか雨", "にわか雪", "雷雨",
    ],
    left: ["{d}日{h}時間", "{h}時間{m}分", "{m}分"],
    refilled: "リセット · 確認中",
    rate_limited: "リクエスト制限 · {}後に再試行",
    rate_limited_soon: "リクエスト制限 · まもなく再試行",
    signed_out: "ログイン切れ · Claude Codeを起動",
    failed: "取得失敗 ({})",
    fans: ["ラジエーター", "ポンプ", "GPU"],
    standby: ["おやすみなさい · またね", "いってきます！", "ちょっと休憩中"],
};

pub static ZH_CN: Strings = Strings {
    code: "zh-CN",
    locale: Locale::zh_CN,
    font: TextFont::YaHei,
    date: "%Y年%-m月%-d日 %A",
    month: "%-m月",
    month_caps: false,
    month_en: false,
    month_names: None,
    weekdays: ["日", "一", "二", "三", "四", "五", "六"],
    loading: "正在获取天气…",
    feels: "体感 {}",
    humidity: "湿度 {}%",
    low: "最低 {}",
    high: "最高 {}",
    rain: "降水 {}%",
    tomorrow: "明天",
    tomorrow_caps: false,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["优", "良", "差", "很差", "危险", "预警", "警报"],
    weather: [
        "晴", "大部晴朗", "多云", "阴", "雾", "毛毛雨", "冻雨", "雨", "大雨", "雪", "大雪", "米雪", "阵雨", "强阵雨", "阵雪", "雷暴",
    ],
    left: ["{d}天{h}小时", "{h}小时{m}分", "{m}分钟"],
    refilled: "已重置 · 确认中",
    rate_limited: "请求受限 · {}后重试",
    rate_limited_soon: "请求受限 · 稍后重试",
    signed_out: "登录已过期 · 请打开 Claude Code",
    failed: "获取失败 ({})",
    fans: ["冷排", "水泵", "显卡"],
    standby: ["晚安 · 回头见", "我出门啦！", "稍作休息"],
};

pub static ZH_TW: Strings = Strings {
    code: "zh-TW",
    locale: Locale::zh_TW,
    font: TextFont::JhengHei,
    date: "%Y年%-m月%-d日 %A",
    month: "%-m月",
    month_caps: false,
    month_en: false,
    month_names: None,
    weekdays: ["日", "一", "二", "三", "四", "五", "六"],
    loading: "正在取得天氣…",
    feels: "體感 {}",
    humidity: "濕度 {}%",
    low: "最低 {}",
    high: "最高 {}",
    rain: "降雨 {}%",
    tomorrow: "明天",
    tomorrow_caps: false,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["良好", "普通", "不良", "非常不良", "危險", "預警", "警報"],
    weather: [
        "晴", "大致晴朗", "多雲", "陰", "霧", "毛毛雨", "凍雨", "雨", "大雨", "雪", "大雪", "米雪", "陣雨", "強陣雨", "陣雪", "雷雨",
    ],
    left: ["{d}天{h}小時", "{h}小時{m}分", "{m}分鐘"],
    refilled: "已重置 · 確認中",
    rate_limited: "請求受限 · {}後重試",
    rate_limited_soon: "請求受限 · 稍後重試",
    signed_out: "登入已過期 · 請開啟 Claude Code",
    failed: "取得失敗 ({})",
    fans: ["水冷排", "水泵", "顯示卡"],
    standby: ["晚安 · 待會見", "我出門囉！", "稍作休息"],
};

pub static ES: Strings = Strings {
    code: "es",
    locale: Locale::es_ES,
    font: TextFont::Noto,
    date: "%A, %-d de %B de %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["Do", "Lu", "Ma", "Mi", "Ju", "Vi", "Sá"],
    loading: "Cargando el tiempo…",
    feels: "Sensación {}",
    humidity: "Humedad {}%",
    low: "Mín {}",
    high: "Máx {}",
    rain: "Lluvia {}%",
    tomorrow: "Mañana",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Buena", "Regular", "Mala", "Muy mala", "Peligrosa", "Aviso", "Alerta"],
    weather: [
        "Despejado", "Poco nuboso", "Parcialmente nuboso", "Cubierto", "Niebla", "Llovizna", "Lluvia helada", "Lluvia",
        "Lluvia fuerte", "Nieve", "Nevada fuerte", "Cinarra", "Chubascos", "Chubascos fuertes", "Chubascos de nieve", "Tormenta",
    ],
    left: ["{d} d {h} h", "{h} h {m} min", "{m} min"],
    refilled: "Reiniciado · comprobando",
    rate_limited: "Límite alcanzado · reintento en {}",
    rate_limited_soon: "Límite alcanzado · reintento pronto",
    signed_out: "Sesión caducada · abre Claude Code",
    failed: "Error ({})",
    fans: ["Radiador", "Bomba", "GPU"],
    standby: ["Buenas noches · hasta pronto", "¡Hasta luego!", "Tomando un descanso"],
};

pub static FR: Strings = Strings {
    code: "fr",
    locale: Locale::fr_FR,
    font: TextFont::Noto,
    date: "%A %-d %B %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["Di", "Lu", "Ma", "Me", "Je", "Ve", "Sa"],
    loading: "Chargement de la météo…",
    feels: "Ressenti {}",
    humidity: "Humidité {}%",
    low: "Min {}",
    high: "Max {}",
    rain: "Pluie {}%",
    tomorrow: "Demain",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Bon", "Moyen", "Mauvais", "Très mauvais", "Dangereux", "Vigilance", "Alerte"],
    weather: [
        "Ciel dégagé", "Plutôt dégagé", "Éclaircies", "Couvert", "Brouillard", "Bruine", "Pluie verglaçante", "Pluie", "Forte pluie",
        "Neige", "Forte neige", "Neige en grains", "Averses", "Fortes averses", "Averses de neige", "Orage",
    ],
    left: ["{d} j {h} h", "{h} h {m} min", "{m} min"],
    refilled: "Réinitialisé · vérification",
    rate_limited: "Limite atteinte · nouvel essai dans {}",
    rate_limited_soon: "Limite atteinte · nouvel essai bientôt",
    signed_out: "Session expirée · ouvrez Claude Code",
    failed: "Échec ({})",
    fans: ["Radiateur", "Pompe", "GPU"],
    standby: ["Bonne nuit · à bientôt", "À plus tard !", "Petite pause"],
};

pub static DE: Strings = Strings {
    code: "de",
    locale: Locale::de_DE,
    font: TextFont::Noto,
    date: "%A, %-d. %B %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"],
    loading: "Wetter wird geladen…",
    feels: "Gefühlt {}",
    humidity: "Feuchte {}%",
    low: "Min {}",
    high: "Max {}",
    rain: "Regen {}%",
    tomorrow: "Morgen",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Gut", "Mäßig", "Schlecht", "Sehr schlecht", "Gefährlich", "Vorwarnung", "Warnung"],
    weather: [
        "Klar", "Überwiegend klar", "Teils bewölkt", "Bedeckt", "Nebel", "Nieselregen", "Gefrierender Regen", "Regen", "Starkregen",
        "Schnee", "Starker Schneefall", "Griesel", "Schauer", "Starke Schauer", "Schneeschauer", "Gewitter",
    ],
    left: ["{d} T {h} Std", "{h} Std {m} Min", "{m} Min"],
    refilled: "Zurückgesetzt · prüfe",
    rate_limited: "Ratenlimit · neuer Versuch in {}",
    rate_limited_soon: "Ratenlimit · gleich neuer Versuch",
    signed_out: "Abgemeldet · Claude Code öffnen",
    failed: "Fehler ({})",
    fans: ["Radiator", "Pumpe", "GPU"],
    standby: ["Gute Nacht · bis bald", "Bis später!", "Kurze Pause"],
};

pub static IT: Strings = Strings {
    code: "it",
    locale: Locale::it_IT,
    font: TextFont::Noto,
    date: "%A %-d %B %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["Do", "Lu", "Ma", "Me", "Gi", "Ve", "Sa"],
    loading: "Caricamento meteo…",
    feels: "Percepita {}",
    humidity: "Umidità {}%",
    low: "Min {}",
    high: "Max {}",
    rain: "Pioggia {}%",
    tomorrow: "Domani",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Buona", "Discreta", "Scarsa", "Molto scarsa", "Pericolosa", "Attenzione", "Allerta"],
    weather: [
        "Sereno", "Poco nuvoloso", "Parz. nuvoloso", "Coperto", "Nebbia", "Pioggerella", "Pioggia gelata", "Pioggia", "Pioggia forte",
        "Neve", "Neve forte", "Neve granulosa", "Rovesci", "Rovesci forti", "Rovesci di neve", "Temporale",
    ],
    left: ["{d} g {h} h", "{h} h {m} min", "{m} min"],
    refilled: "Azzerato · verifica",
    rate_limited: "Limite raggiunto · riprovo tra {}",
    rate_limited_soon: "Limite raggiunto · riprovo a breve",
    signed_out: "Sessione scaduta · apri Claude Code",
    failed: "Errore ({})",
    fans: ["Radiatore", "Pompa", "GPU"],
    standby: ["Buonanotte · a presto", "A dopo!", "Una piccola pausa"],
};

pub static PT: Strings = Strings {
    code: "pt",
    locale: Locale::pt_BR,
    font: TextFont::Noto,
    date: "%A, %-d de %B de %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["Do", "Se", "Te", "Qu", "Qu", "Se", "Sá"],
    loading: "Carregando o clima…",
    feels: "Sensação {}",
    humidity: "Umidade {}%",
    low: "Mín {}",
    high: "Máx {}",
    rain: "Chuva {}%",
    tomorrow: "Amanhã",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Boa", "Moderada", "Ruim", "Muito ruim", "Perigosa", "Atenção", "Alerta"],
    weather: [
        "Céu limpo", "Predomínio de sol", "Parcialmente nublado", "Nublado", "Nevoeiro", "Garoa", "Chuva congelante", "Chuva",
        "Chuva forte", "Neve", "Neve forte", "Grãos de neve", "Pancadas", "Pancadas fortes", "Pancadas de neve", "Trovoada",
    ],
    left: ["{d} d {h} h", "{h} h {m} min", "{m} min"],
    refilled: "Reiniciado · verificando",
    rate_limited: "Limite atingido · nova tentativa em {}",
    rate_limited_soon: "Limite atingido · nova tentativa em breve",
    signed_out: "Sessão expirada · abra o Claude Code",
    failed: "Falha ({})",
    fans: ["Radiador", "Bomba", "GPU"],
    standby: ["Boa noite · até logo", "Até mais!", "Fazendo uma pausa"],
};

pub static RU: Strings = Strings {
    code: "ru",
    locale: Locale::ru_RU,
    font: TextFont::Noto,
    date: "%A, %-d %B %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: Some(["Январь", "Февраль", "Март", "Апрель", "Май", "Июнь", "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь"]),
    weekdays: ["Вс", "Пн", "Вт", "Ср", "Чт", "Пт", "Сб"],
    loading: "Загрузка погоды…",
    feels: "Ощущается {}",
    humidity: "Влажность {}%",
    low: "Мин {}",
    high: "Макс {}",
    rain: "Осадки {}%",
    tomorrow: "Завтра",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Хорошо", "Умеренно", "Плохо", "Очень плохо", "Опасно", "Внимание", "Тревога"],
    weather: [
        "Ясно", "Малооблачно", "Переменная облачность", "Пасмурно", "Туман", "Морось", "Ледяной дождь", "Дождь", "Сильный дождь",
        "Снег", "Сильный снег", "Снежная крупа", "Ливень", "Сильный ливень", "Снегопад", "Гроза",
    ],
    left: ["{d} д {h} ч", "{h} ч {m} мин", "{m} мин"],
    refilled: "Сброшено · проверка",
    rate_limited: "Лимит запросов · повтор через {}",
    rate_limited_soon: "Лимит запросов · скоро повтор",
    signed_out: "Сеанс истёк · откройте Claude Code",
    failed: "Ошибка ({})",
    fans: ["Радиатор", "Помпа", "GPU"],
    standby: ["Спокойной ночи · до встречи", "До скорого!", "Небольшой перерыв"],
};

pub static PL: Strings = Strings {
    code: "pl",
    locale: Locale::pl_PL,
    font: TextFont::Noto,
    date: "%A, %-d %B %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: Some([
        "Styczeń", "Luty", "Marzec", "Kwiecień", "Maj", "Czerwiec", "Lipiec", "Sierpień", "Wrzesień", "Październik", "Listopad", "Grudzień",
    ]),
    weekdays: ["Nd", "Pn", "Wt", "Śr", "Cz", "Pt", "So"],
    loading: "Wczytywanie pogody…",
    feels: "Odczuwalna {}",
    humidity: "Wilgotność {}%",
    low: "Min {}",
    high: "Maks {}",
    rain: "Opady {}%",
    tomorrow: "Jutro",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Dobra", "Umiarkowana", "Zła", "Bardzo zła", "Niebezpieczna", "Ostrzeżenie", "Alarm"],
    weather: [
        "Bezchmurnie", "Przeważnie pogodnie", "Częściowe zachmurzenie", "Pochmurno", "Mgła", "Mżawka", "Marznący deszcz", "Deszcz",
        "Ulewa", "Śnieg", "Intensywny śnieg", "Krupa śnieżna", "Przelotne opady", "Silne opady", "Przelotny śnieg", "Burza",
    ],
    left: ["{d} d {h} h", "{h} h {m} min", "{m} min"],
    refilled: "Odnowiono · sprawdzanie",
    rate_limited: "Limit zapytań · ponowienie za {}",
    rate_limited_soon: "Limit zapytań · zaraz ponowienie",
    signed_out: "Sesja wygasła · otwórz Claude Code",
    failed: "Błąd ({})",
    fans: ["Chłodnica", "Pompa", "GPU"],
    standby: ["Dobranoc · do zobaczenia", "Na razie!", "Krótka przerwa"],
};

pub static TR: Strings = Strings {
    code: "tr",
    locale: Locale::tr_TR,
    font: TextFont::Noto,
    date: "%-d %B %Y %A",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["Pz", "Pt", "Sa", "Ça", "Pe", "Cu", "Ct"],
    loading: "Hava durumu yükleniyor…",
    feels: "Hissedilen {}",
    humidity: "Nem %{}",
    low: "En düşük {}",
    high: "En yüksek {}",
    rain: "Yağış %{}",
    tomorrow: "Yarın",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["İyi", "Orta", "Kötü", "Çok kötü", "Tehlikeli", "Uyarı", "Alarm"],
    weather: [
        "Açık", "Çoğunlukla açık", "Parçalı bulutlu", "Kapalı", "Sis", "Çisenti", "Dondurucu yağmur", "Yağmur", "Kuvvetli yağmur",
        "Kar", "Yoğun kar", "Kar taneleri", "Sağanak", "Kuvvetli sağanak", "Kar sağanağı", "Fırtına",
    ],
    left: ["{d} g {h} sa", "{h} sa {m} dk", "{m} dk"],
    refilled: "Sıfırlandı · kontrol ediliyor",
    rate_limited: "İstek sınırı · {} sonra tekrar",
    rate_limited_soon: "İstek sınırı · birazdan tekrar",
    signed_out: "Oturum kapandı · Claude Code'u açın",
    failed: "Hata ({})",
    fans: ["Radyatör", "Pompa", "GPU"],
    standby: ["İyi geceler · görüşürüz", "Sonra görüşürüz!", "Kısa bir mola"],
};

pub static NL: Strings = Strings {
    code: "nl",
    locale: Locale::nl_NL,
    font: TextFont::Noto,
    date: "%A %-d %B %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["Zo", "Ma", "Di", "Wo", "Do", "Vr", "Za"],
    loading: "Weer laden…",
    feels: "Voelt als {}",
    humidity: "Vocht {}%",
    low: "Min {}",
    high: "Max {}",
    rain: "Regen {}%",
    tomorrow: "Morgen",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Goed", "Matig", "Slecht", "Zeer slecht", "Gevaarlijk", "Waarschuwing", "Alarm"],
    weather: [
        "Helder", "Overwegend helder", "Half bewolkt", "Bewolkt", "Mist", "Motregen", "IJzel", "Regen", "Zware regen", "Sneeuw",
        "Zware sneeuw", "Korrelsneeuw", "Buien", "Zware buien", "Sneeuwbuien", "Onweer",
    ],
    left: ["{d} d {h} u", "{h} u {m} min", "{m} min"],
    refilled: "Gereset · controleren",
    rate_limited: "Limiet bereikt · opnieuw over {}",
    rate_limited_soon: "Limiet bereikt · zo opnieuw",
    signed_out: "Afgemeld · open Claude Code",
    failed: "Mislukt ({})",
    fans: ["Radiator", "Pomp", "GPU"],
    standby: ["Welterusten · tot snel", "Tot later!", "Even pauze"],
};

pub static VI: Strings = Strings {
    code: "vi",
    locale: Locale::vi_VN,
    font: TextFont::Noto,
    date: "%A, %-d/%-m/%Y",
    month: "Tháng %-m",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["CN", "T2", "T3", "T4", "T5", "T6", "T7"],
    loading: "Đang tải thời tiết…",
    feels: "Cảm giác {}",
    humidity: "Độ ẩm {}%",
    low: "Thấp {}",
    high: "Cao {}",
    rain: "Mưa {}%",
    tomorrow: "Ngày mai",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Tốt", "Trung bình", "Kém", "Xấu", "Nguy hại", "Cảnh báo", "Báo động"],
    weather: [
        "Trời quang", "Ít mây", "Mây rải rác", "Nhiều mây", "Sương mù", "Mưa phùn", "Mưa băng", "Mưa", "Mưa to", "Tuyết", "Tuyết dày",
        "Mưa tuyết hạt", "Mưa rào", "Mưa rào mạnh", "Mưa tuyết rào", "Dông",
    ],
    left: ["{d} ngày {h} giờ", "{h} giờ {m} phút", "{m} phút"],
    refilled: "Đã đặt lại · đang kiểm tra",
    rate_limited: "Giới hạn yêu cầu · thử lại sau {}",
    rate_limited_soon: "Giới hạn yêu cầu · sắp thử lại",
    signed_out: "Hết phiên · mở Claude Code",
    failed: "Lỗi ({})",
    fans: ["Tản nhiệt", "Bơm", "GPU"],
    standby: ["Chúc ngủ ngon · hẹn gặp lại", "Hẹn gặp lại!", "Nghỉ một chút"],
};

pub static ID: Strings = Strings {
    code: "id",
    locale: Locale::id_ID,
    font: TextFont::Noto,
    date: "%A, %-d %B %Y",
    month: "%B",
    month_caps: true,
    month_en: false,
    month_names: None,
    weekdays: ["Mg", "Sn", "Sl", "Rb", "Km", "Jm", "Sb"],
    loading: "Memuat cuaca…",
    feels: "Terasa {}",
    humidity: "Lembap {}%",
    low: "Min {}",
    high: "Maks {}",
    rain: "Hujan {}%",
    tomorrow: "Besok",
    tomorrow_caps: true,
    pm25: "PM2.5",
    pm10: "PM10",
    air: ["Baik", "Sedang", "Buruk", "Sangat buruk", "Berbahaya", "Waspada", "Siaga"],
    weather: [
        "Cerah", "Cerah berawan", "Berawan sebagian", "Mendung", "Kabut", "Gerimis", "Hujan beku", "Hujan", "Hujan lebat", "Salju",
        "Salju lebat", "Butiran salju", "Hujan lokal", "Hujan lokal lebat", "Salju lokal", "Badai petir",
    ],
    left: ["{d} h {h} j", "{h} j {m} m", "{m} m"],
    refilled: "Direset · memeriksa",
    rate_limited: "Batas permintaan · coba lagi dalam {}",
    rate_limited_soon: "Batas permintaan · segera coba lagi",
    signed_out: "Sesi habis · buka Claude Code",
    failed: "Gagal ({})",
    fans: ["Radiator", "Pompa", "GPU"],
    standby: ["Selamat malam · sampai jumpa", "Sampai nanti!", "Istirahat sebentar"],
};
