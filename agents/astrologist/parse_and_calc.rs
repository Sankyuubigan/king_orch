use chrono::{DateTime, Duration, TimeZone, Utc};
use libswisseph_sys::*;
use std::ffi::CString;
use std::fs;
use std::os::raw::c_char;
use std::str::FromStr;

// ============================================================================
// 1. КОНСТАНТЫ И СПРАВОЧНИКИ ДЖЙОТИШ
// ============================================================================

/// Длина года Даши согласно скриншоту "Year with 360 tithis" (~354.367068 дней)
/// Если переключишь в JHora на другую строку со скрина, просто раскомментируй нужную:
const DASHA_YEAR_DAYS: f64 = 354.3670680; // Year with 360 tithis (1 титхи = 0.9843529667 сут.)
// const DASHA_YEAR_DAYS: f64 = 365.242190; // Mean tropical solar year
// const DASHA_YEAR_DAYS: f64 = 365.256364; // Mean sidereal solar year
// const DASHA_YEAR_DAYS: f64 = 360.000000; // Savana year (360 days)

/// Код Айанамши в Swiss Ephemeris: 21 = SE_SIDM_SURYASIDDHANTA
const AYANAMSHA_MODE: i32 = SE_SIDM_SURYASIDDHANTA as i32;

const SIGNS: [&str; 12] = [
    "Овен", "Телец", "Близнецы", "Рак", "Лев", "Дева",
    "Весы", "Скорпион", "Стрелец", "Козерог", "Водолей", "Рыбы",
];

const NAKSHATRAS: [&str; 27] = [
    "Ашвини", "Бхарани", "Криттика", "Рохини", "Мригашира", "Ардра",
    "Пунарвасу", "Пушья", "Ашлеша", "Магха", "Пурва Пхалгуни", "Уттара Пхалгуни",
    "Хаста", "Читра", "Свати", "Вишакха", "Анурадха", "Джйештха",
    "Мула", "Пурва Ашадха", "Уттара Ашадха", "Шравана", "Дхаништха", "Шатабхиша",
    "Пурва Бхадрапада", "Уттара Бхадрапада", "Ревати",
];

const DASHA_LORDS: [&str; 9] = [
    "Кету", "Венера", "Солнце", "Луна", "Марс", "Раху", "Юпитер", "Сатурн", "Меркурий",
];

const DASHA_YEARS: [f64; 9] = [7.0, 20.0, 6.0, 10.0, 7.0, 18.0, 16.0, 19.0, 17.0];

// ============================================================================
// 2. СТРУКТУРЫ ДАННЫХ (ТО, В ЧЕМ ХРАНЯТСЯ ВСЕ ПЕРЕМЕННЫЕ КАРТЫ)
// ============================================================================

/// Исходные данные из файла .jhd
#[derive(Debug, Clone)]
pub struct JhdInput {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour_local: f64,
    pub timezone_offset: f64, // часы относительно UTC (например, +2.0)
    pub longitude_east: f64,  // градусы (+ Восток, - Запад)
    pub latitude_north: f64,  // градусы (+ Север, - Юг)
    pub altitude: f64,
    pub city: String,
    pub country: String,
    pub birth_utc: DateTime<Utc>,
    pub julian_day_ut: f64,
}

/// Полная информация о планете или Лагне
#[derive(Debug, Clone)]
pub struct PlanetInfo {
    pub name: String,
    pub longitude: f64,          // Сидерическая долгота 0..360
    pub is_retrograde: bool,     // Ретроградность
    pub sign_d1_idx: usize,      // Индекс знака D1 (0..11)
    pub sign_d1_name: String,    // Название знака D1
    pub degree_in_sign: f64,     // Градус внутри знака (0..30)
    pub deg_min_sec: (u32, u32, f64), // Градусы, минуты, секунды внутри знака
    pub nakshatra_idx: usize,    // Индекс накшатры (0..26)
    pub nakshatra_name: String,  // Название накшатры
    pub pada: u8,                // Пада накшатры (1..4)
    pub house_d1: u8,            // Дом в Раши D1 (1..12)
    pub sign_d9_idx: usize,      // Индекс знака в Навамше D9 (0..11)
    pub sign_d9_name: String,    // Название знака в Навамше D9
    pub house_d9: u8,            // Дом в Навамше D9 (1..12)
}

/// Информация об одном доме (Бхаве)
#[derive(Debug, Clone)]
pub struct HouseInfo {
    pub house_number: u8,        // 1..12
    pub sign_idx: usize,         // Знак дома (0..11)
    pub sign_name: String,       // Название знака
    pub planets: Vec<String>,    // Список планет в этом доме
}

/// Универсальный узел периода Вимшоттари Даши (для MD, AD, PD, SD, PAD)
#[derive(Debug, Clone)]
pub struct DashaPeriod {
    pub level: u8,               // 1=MD, 2=AD, 3=PD, 4=SD, 5=PAD
    pub lord_idx: usize,         // Индекс планеты-управителя (0..8)
    pub lord_name: String,       // Имя управителя
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    pub sub_periods: Vec<DashaPeriod>, // Вложенные подпериоды
}

/// Главная структура всей натальной карты (держит всё в себе)
#[derive(Debug, Clone)]
pub struct VedicChart {
    pub input: JhdInput,
    pub ayanamsha_value: f64,
    pub lagna: PlanetInfo,
    pub planets: Vec<PlanetInfo>, // 9 грах: Солнце..Кету
    pub houses_d1: Vec<HouseInfo>, // 12 домов Раши (D1)
    pub houses_d9: Vec<HouseInfo>, // 12 домов Навамши (D9)
    pub vimshottari_dasha: Vec<DashaPeriod>, // Дерево MD -> AD -> PD -> SD -> PAD
}

// ============================================================================
// 3. ПАРСЕР ФАЙЛА .JHD И РАСЧЕТ ЮЛИАНСКОГО ДНЯ
// ============================================================================

/// Перевод формата JHora (ГГ.ММСС -> Десятичные градусы/часы)
fn parse_jhora_dms(val: f64) -> f64 {
    let sign = if val < 0.0 { -1.0 } else { 1.0 };
    let abs_val = val.abs();
    let deg = abs_val.floor();
    let min_sec = (abs_val - deg) * 100.0;
    let min = (min_sec + 1e-9).floor();
    let sec = (min_sec - min) * 100.0;
    sign * (deg + min / 60.0 + sec / 3600.0)
}

/// Перевод десятичных градусов в (Градусы, Минуты, Секунды)
fn to_dms(deg_decimal: f64) -> (u32, u32, f64) {
    let d = deg_decimal.floor() as u32;
    let m_full = (deg_decimal - d as f64) * 60.0;
    let m = m_full.floor() as u32;
    let s = (m_full - m as f64) * 60.0;
    (d, m, s)
}

impl JhdInput {
    pub fn from_jhd_str(content: &str) -> Result<Self, String> {
        let lines: Vec<&str> = content.lines().map(|l| l.trim()).collect();
        if lines.len() < 14 {
            return Err("Файл .jhd поврежден или содержит меньше 14 строк".to_string());
        }

        let month = u32::from_str(lines[0]).map_err(|e| e.to_string())?;
        let day = u32::from_str(lines[1]).map_err(|e| e.to_string())?;
        let year = i32::from_str(lines[2]).map_err(|e| e.to_string())?;

        let raw_time = f64::from_str(lines[3]).map_err(|e| e.to_string())?;
        let raw_tz = f64::from_str(lines[4]).map_err(|e| e.to_string())?;
        let raw_lon = f64::from_str(lines[5]).map_err(|e| e.to_string())?;
        let raw_lat = f64::from_str(lines[6]).map_err(|e| e.to_string())?;
        let altitude = f64::from_str(lines[7]).unwrap_or(0.0);

        let hour_local = parse_jhora_dms(raw_time);
        let timezone_offset = -parse_jhora_dms(raw_tz); // В JHora восток со знаком минус
        let longitude_east = -parse_jhora_dms(raw_lon); // В JHora восток со знаком минус
        let latitude_north = parse_jhora_dms(raw_lat);

        let city = lines[12].replace('^', " ");
        let country = lines[13].replace('^', " ");

        // Вычисляем точное время UTC
        let hour_ut = hour_local - timezone_offset;
        let jd_ut = unsafe {
            swe_julday(
                year,
                month as i32,
                day as i32,
                hour_ut,
                SE_GREG_CAL as i32,
            )
        };

        // Создаем DateTime<Utc> для расчетов календаря Даш
        let local_sec = (hour_local * 3600.0).round() as i64;
        let tz_sec = (timezone_offset * 3600.0).round() as i64;
        let base_date = Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap();
        let birth_utc = base_date + Duration::seconds(local_sec - tz_sec);

        Ok(JhdInput {
            year,
            month,
            day,
            hour_local,
            timezone_offset,
            longitude_east,
            latitude_north,
            altitude,
            city,
            country,
            birth_utc,
            julian_day_ut: jd_ut,
        })
    }
}

// ============================================================================
// 4. ЯДРО РАСЧЕТА КАРТЫ (SWISS EPHEMERIS + МАТЕМАТИКА ДЖЙОТИШ)
// ============================================================================

fn build_planet_info(
    name: &str,
    lon: f64,
    is_retro: bool,
    lagna_d1_sign: usize,
    lagna_d9_sign: usize,
) -> PlanetInfo {
    let norm_lon = lon.rem_euclid(360.0);
    let sign_d1_idx = (norm_lon / 30.0).floor() as usize;
    let degree_in_sign = norm_lon % 30.0;
    let deg_min_sec = to_dms(degree_in_sign);

    let nakshatra_idx = ((norm_lon * 27.0) / 360.0).floor() as usize;
    let total_pada = ((norm_lon * 108.0) / 360.0).floor() as usize;
    let pada = ((total_pada % 4) + 1) as u8;

    // Навамша (D9): каждая пада накшатры = 1 знак D9 начиная от Овна
    let sign_d9_idx = total_pada % 12;

    // Дома (Whole Sign / Раши = Дом)
    let house_d1 = (((sign_d1_idx + 12 - lagna_d1_sign) % 12) + 1) as u8;
    let house_d9 = (((sign_d9_idx + 12 - lagna_d9_sign) % 12) + 1) as u8;

    PlanetInfo {
        name: name.to_string(),
        longitude: norm_lon,
        is_retrograde: is_retro,
        sign_d1_idx,
        sign_d1_name: SIGNS[sign_d1_idx].to_string(),
        degree_in_sign,
        deg_min_sec,
        nakshatra_idx,
        nakshatra_name: NAKSHATRAS[nakshatra_idx].to_string(),
        pada,
        house_d1,
        sign_d9_idx,
        sign_d9_name: SIGNS[sign_d9_idx].to_string(),
        house_d9,
    }
}

/// Рекурсивная функция генерации подпериодов Вимшоттари (AD -> PD -> SD -> PAD)
fn generate_sub_periods(
    current_level: u8,
    max_level: u8,
    start_lord_idx: usize,
    parent_start: DateTime<Utc>,
    parent_duration_days: f64,
) -> Vec<DashaPeriod> {
    if current_level > max_level {
        return Vec::new();
    }

    let mut periods = Vec::with_capacity(9);
    let mut cursor_date = parent_start;

    for i in 0..9 {
        let lord_idx = (start_lord_idx + i) % 9;
        let sub_duration_days = parent_duration_days * (DASHA_YEARS[lord_idx] / 120.0);
        let duration_ms = (sub_duration_days * 86_400_000.0).round() as i64;
        let end_date = cursor_date + Duration::milliseconds(duration_ms);

        let sub_periods = generate_sub_periods(
            current_level + 1,
            max_level,
            lord_idx,
            cursor_date,
            sub_duration_days,
        );

        periods.push(DashaPeriod {
            level: current_level,
            lord_idx,
            lord_name: DASHA_LORDS[lord_idx].to_string(),
            start_date: cursor_date,
            end_date,
            sub_periods,
        });

        cursor_date = end_date;
    }

    periods
}

/// Расчет полного дерева Вимшоттари Даши от долготы Луны (max_level = 5 для PAD)
pub fn calculate_vimshottari(
    moon_lon: f64,
    birth_utc: DateTime<Utc>,
    max_level: u8,
) -> Vec<DashaPeriod> {
    let norm_moon = moon_lon.rem_euclid(360.0);
    let nak_span = 360.0 / 27.0; // 13.333333333333334 градусов
    let nak_idx = (norm_moon / nak_span).floor() as usize;
    let first_lord_idx = nak_idx % 9;

    // Доля пройденной накшатры на момент рождения
    let deg_in_nak = norm_moon % nak_span;
    let fraction_passed = deg_in_nak / nak_span;

    // Полная длительность первой Махадаши в днях (по настройке 360 титхи)
    let first_md_full_days = DASHA_YEARS[first_lord_idx] * DASHA_YEAR_DAYS;
    let elapsed_days = first_md_full_days * fraction_passed;

    // Теоретическое начало первой Махадаши (до рождения), чтобы все AD/PD/SD/PAD шли точно!
    let elapsed_ms = (elapsed_days * 86_400_000.0).round() as i64;
    let mut cursor_date = birth_utc - Duration::milliseconds(elapsed_ms);

    let mut mahadashas = Vec::with_capacity(9);

    for i in 0..9 {
        let lord_idx = (first_lord_idx + i) % 9;
        let md_days = DASHA_YEARS[lord_idx] * DASHA_YEAR_DAYS;
        let md_ms = (md_days * 86_400_000.0).round() as i64;
        let end_date = cursor_date + Duration::milliseconds(md_ms);

        let sub_periods = generate_sub_periods(2, max_level, lord_idx, cursor_date, md_days);

        mahadashas.push(DashaPeriod {
            level: 1,
            lord_idx,
            lord_name: DASHA_LORDS[lord_idx].to_string(),
            start_date: cursor_date,
            end_date,
            sub_periods,
        });

        cursor_date = end_date;
    }

    mahadashas
}

/// Главная функция построения всей карты из JhdInput
pub fn calculate_chart(input: JhdInput, dasha_max_level: u8) -> Result<VedicChart, String> {
    unsafe {
        // Путь к файлам эфемерид (если пусто — используется встроенный алгоритм Moshier)
        let ephe_path = CString::new("").unwrap();
        swe_set_ephe_path(ephe_path.as_ptr());

        // Устанавливаем Айанамшу: Sri Surya Siddhanta
        swe_set_sid_mode(AYANAMSHA_MODE, 0.0, 0.0);

        let ayanamsha_value = swe_get_ayanamsa_ut(input.julian_day_ut);

        // 1. Вычисляем Лагну (Асцендент)
        let mut cusps = [0.0f64; 13];
        let mut ascmc = [0.0f64; 10];
        swe_houses_ex(
            input.julian_day_ut,
            SEFLG_SIDEREAL as i32,
            input.latitude_north,
            input.longitude_east,
            b'W' as i32, // Whole Sign
            cusps.as_mut_ptr(),
            ascmc.as_mut_ptr(),
        );
        let lagna_lon = ascmc[0].rem_euclid(360.0);
        let lagna_d1_sign = (lagna_lon / 30.0).floor() as usize;
        let lagna_d9_sign = (((lagna_lon * 108.0) / 360.0).floor() as usize) % 12;

        let lagna = build_planet_info("Лагна (Asc)", lagna_lon, false, lagna_d1_sign, lagna_d9_sign);

        // 2. Вычисляем 7 планет + Раху и Кету
        let bodies = [
            (SE_SUN as i32, "Солнце"),
            (SE_MOON as i32, "Луна"),
            (SE_MARS as i32, "Марс"),
            (SE_MERCURY as i32, "Меркурий"),
            (SE_JUPITER as i32, "Юпитер"),
            (SE_VENUS as i32, "Венера"),
            (SE_SATURN as i32, "Сатурн"),
            (SE_MEAN_NODE as i32, "Раху"), // Или SE_TRUE_NODE для истинного узла
        ];

        let mut planets = Vec::with_capacity(9);
        let mut rahu_lon = 0.0;
        let mut moon_lon = 0.0;

        let flags = (SEFLG_SIDEREAL | SEFLG_SPEED) as i32;

        for (ipl, name) in bodies.iter() {
            let mut xx = [0.0f64; 6];
            let mut serr = [0 as c_char; 256];
            let ret = swe_calc_ut(
                input.julian_day_ut,
                *ipl,
                flags,
                xx.as_mut_ptr(),
                serr.as_mut_ptr(),
            );
            if ret < 0 {
                return Err(format!("Ошибка расчета эфемерид для {}", name));
            }

            let lon = xx[0].rem_euclid(360.0);
            let speed = xx[3];
            let is_retro = if *ipl == SE_MEAN_NODE as i32 {
                true
            } else {
                speed < 0.0
            };

            if *ipl == SE_MOON as i32 {
                moon_lon = lon;
            }
            if *ipl == SE_MEAN_NODE as i32 {
                rahu_lon = lon;
            }

            planets.push(build_planet_info(
                name,
                lon,
                is_retro,
                lagna_d1_sign,
                lagna_d9_sign,
            ));
        }

        // Добавляем Кету (ровно 180 градусов от Раху)
        let ketu_lon = (rahu_lon + 180.0).rem_euclid(360.0);
        planets.push(build_planet_info(
            "Кету",
            ketu_lon,
            true,
            lagna_d1_sign,
            lagna_d9_sign,
        ));

        // 3. Собираем структуры Домов для D1 и D9
        let mut houses_d1 = Vec::with_capacity(12);
        let mut houses_d9 = Vec::with_capacity(12);

        for h in 1..=12u8 {
            let sign_d1 = (lagna_d1_sign + (h as usize) - 1) % 12;
            let sign_d9 = (lagna_d9_sign + (h as usize) - 1) % 12;

            let p_d1: Vec<String> = planets
                .iter()
                .filter(|p| p.house_d1 == h)
                .map(|p| p.name.clone())
                .collect();

            let p_d9: Vec<String> = planets
                .iter()
                .filter(|p| p.house_d9 == h)
                .map(|p| p.name.clone())
                .collect();

            houses_d1.push(HouseInfo {
                house_number: h,
                sign_idx: sign_d1,
                sign_name: SIGNS[sign_d1].to_string(),
                planets: p_d1,
            });

            houses_d9.push(HouseInfo {
                house_number: h,
                sign_idx: sign_d9,
                sign_name: SIGNS[sign_d9].to_string(),
                planets: p_d9,
            });
        }

        // 4. Вычисляем дерево Вимшоттари Даши (до уровня dasha_max_level: 5 = PAD)
        let vimshottari_dasha = calculate_vimshottari(moon_lon, input.birth_utc, dasha_max_level);

        Ok(VedicChart {
            input,
            ayanamsha_value,
            lagna,
            planets,
            houses_d1,
            houses_d9,
            vimshottari_dasha,
        })
    }
}

// ============================================================================
// 5. ПОИСК АКТИВНЫХ ПЕРИОДОВ ДАШИ НА ЛЮБУЮ ДАТУ И ПРИМЕР ЗАПУСКА
// ============================================================================

/// Возвращает цепочку активных периодов [MD, AD, PD, SD, PAD] на заданный момент времени
pub fn find_active_dasha_chain<'a>(
    tree: &'a [DashaPeriod],
    target_date: DateTime<Utc>,
) -> Vec<&'a DashaPeriod> {
    let mut chain = Vec::new();
    let mut current_slice = tree;

    while let Some(period) = current_slice
        .iter()
        .find(|p| target_date >= p.start_date && target_date < p.end_date)
    {
        chain.push(period);
        current_slice = &period.sub_periods;
    }

    chain
}

fn main() {
    // Можно читать с диска:
    // let jhd_content = fs::read_to_string("Kirill Grafov Telega.jhd").expect("Не удалось прочитать файл");
    
    // Или передать содержимое твоего файла напрямую для теста:
    let jhd_content = "\
12
22
2000
8.000000000000000
-2.000000
-34.150000
46.250000
0.000000
-2.000000
-3.000000
0
232
Krivoy^Rog
Ukraine
1
1013.250000
20.000000
1";

    // 1. Парсим .jhd
    let input = JhdInput::from_jhd_str(jhd_content).expect("Ошибка парсинга JHD");

    // 2. Считаем всю карту (5 = считаем все уровни вплоть до Прана-даши PAD)
    // ВСЯ ИНФОРМАЦИЯ ТЕПЕРЬ ЛЕЖИТ В ПЕРЕМЕННОЙ `chart`!
    let chart = calculate_chart(input, 5).expect("Ошибка расчета карты");

    // 3. Выводим данные из переменных:
    println!("=== ДАННЫЕ КАРТЫ ===");
    println!(
        "Город: {}, {} | Дата (UTC): {}",
        chart.input.city, chart.input.country, chart.input.birth_utc
    );
    let (ay_d, ay_m, ay_s) = to_dms(chart.ayanamsha_value);
    println!("Айанамша (Sri Surya Siddhanta): {}° {}' {:.2}\"", ay_d, ay_m, ay_s);

    println!("\n=== 1 & 2. ПЛАНЕТЫ: ЗНАКИ (D1), ГРАДУСЫ, НАКШАТРЫ, ПАДЫ И НАВАМША (D9) ===");
    let all_points = std::iter::once(&chart.lagna).chain(chart.planets.iter());
    for p in all_points {
        let retro_mark = if p.is_retrograde { "(R)" } else { "   " };
        println!(
            "{:<11} {} | D1: {:<8} {:02}°{:02}'{:04.1}\" (Дом {:2}) | Накшатра: {:<17} Пада {} | D9: {:<8} (Дом {:2})",
            p.name,
            retro_mark,
            p.sign_d1_name,
            p.deg_min_sec.0,
            p.deg_min_sec.1,
            p.deg_min_sec.2,
            p.house_d1,
            p.nakshatra_name,
            p.pada,
            p.sign_d9_name,
            p.house_d9
        );
    }

    println!("\n=== 3. ДОМА В РАШИ (D1) И ПЛАНЕТЫ В НИХ ===");
    for h in &chart.houses_d1 {
        println!(
            "Дом {:2} ({:<8}): {:?}",
            h.house_number, h.sign_name, h.planets
        );
    }

    println!("\n=== 4. ВИМШОТТАРИ ДАША (СПИСОК МАХАДАШ - MD) ===");
    for md in &chart.vimshottari_dasha {
        println!(
            "MD {:<9}: {}  -->  {}",
            md.lord_name,
            md.start_date.format("%Y-%m-%d"),
            md.end_date.format("%Y-%m-%d")
        );
    }

    // Пример: смотрим все 5 уровней (MD -> AD -> PD -> SD -> PAD) на текущий момент
    let now = Utc::now();
    let active_chain = find_active_dasha_chain(&chart.vimshottari_dasha, now);
    let level_names = ["MD (Махадаша)", "AD (Антардаша)", "PD (Пратьянтардаша)", "SD (Сукшма-даша)", "PAD (Прана-даша)"];

    println!("\n=== АКТИВНЫЕ ПЕРИОДЫ (MD-AD-PD-SD-PAD) НА {} ===", now.format("%Y-%m-%d %H:%M UTC"));
    for (idx, period) in active_chain.iter().enumerate() {
        println!(
            "{:<22}: {:<9} ({} .. {})",
            level_names[idx],
            period.lord_name,
            period.start_date.format("%Y-%m-%d %H:%M"),
            period.end_date.format("%Y-%m-%d %H:%M")
        );
    }
}