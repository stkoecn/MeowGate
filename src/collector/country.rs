/// Country/region inference from node names and probe results.
pub struct CountryInfer;

/// Regional indicator emoji -> ISO code handled generically; keyword table below.
const KEYWORDS: &[(&str, &str)] = &[
    // (keyword lowercase, ISO-3166 alpha-2 code)
    ("美国", "US"), ("los angeles", "US"), ("new york", "US"), ("san jose", "US"),
    ("chicago", "US"), ("seattle", "US"), ("dallas", "US"), ("ashburn", "US"),
    ("🇺", "US"), ("us-", "US"), ("-us", "US"), ("usa", "US"),
    ("日本", "JP"), ("tokyo", "JP"), ("osaka", "JP"), ("🇯", "JP"), ("jp-", "JP"), ("-jp", "JP"),
    ("香港", "HK"), ("hong kong", "HK"), ("🇭", "HK"), ("hk-", "HK"), ("-hk", "HK"),
    ("新加坡", "SG"), ("singapore", "SG"), ("🇸🇬", "SG"), ("sg-", "SG"), ("-sg", "SG"),
    ("韩国", "KR"), ("seoul", "KR"), ("🇰", "KR"), ("kr-", "KR"), ("-kr", "KR"),
    ("台湾", "TW"), ("台北", "TW"), ("🇹🇼", "TW"), ("tw-", "TW"),
    ("英国", "GB"), ("london", "GB"), ("🇬🇧", "GB"), ("uk", "GB"), ("gb-", "GB"),
    ("德国", "DE"), ("frankfurt", "DE"), ("🇩", "DE"), ("de-", "DE"),
    ("法国", "FR"), ("paris", "FR"), ("🇫🇷", "FR"), ("fr-", "FR"),
    ("荷兰", "NL"), ("amsterdam", "NL"), ("🇳🇱", "NL"), ("nl-", "NL"),
    ("加拿大", "CA"), ("🇨", "CA"), ("ca-", "CA"), ("toronto", "CA"), ("vancouver", "CA"),
    ("澳大利亚", "AU"), ("sydney", "AU"), ("🇦", "AU"), ("au-", "AU"),
    ("俄罗斯", "RU"), ("moscow", "RU"), ("🇷🇺", "RU"), ("ru-", "RU"),
    ("土耳其", "TR"), ("istanbul", "TR"), ("🇹🇷", "TR"),
    ("巴西", "BR"), ("sao paulo", "BR"), ("🇧🇷", "BR"),
    ("印度", "IN"), ("mumbai", "IN"), ("🇮", "IN"),
    ("越南", "VN"), ("🇻🇳", "VN"), ("hanoi", "VN"), ("hcmc", "VN"),
    ("泰国", "TH"), ("bangkok", "TH"), ("🇹🇭", "TH"),
    ("马来西亚", "MY"), ("kuala lumpur", "MY"), ("🇲🇾", "MY"),
    ("印尼", "ID"), ("印度尼西亚", "ID"), ("jakarta", "ID"), ("🇮🇩", "ID"),
    ("阿根廷", "AR"), ("🇦🇷", "AR"),
    ("智利", "CL"), ("🇨🇱", "CL"),
    ("墨西哥", "MX"), ("🇲🇽", "MX"),
    ("阿联酋", "AE"), ("uae", "AE"), ("dubai", "AE"), ("🇦🇪", "AE"),
    ("沙特", "SA"), ("🇸🇦", "SA"),
    ("以色列", "IL"), ("🇮🇱", "IL"),
    ("乌克兰", "UA"), ("🇺🇦", "UA"),
    ("波兰", "PL"), ("🇵🇱", "PL"), ("warsaw", "PL"),
    ("意大利", "IT"), ("🇮🇹", "IT"), ("rome", "IT"), ("milan", "IT"),
    ("西班牙", "ES"), ("🇪🇸", "ES"), ("madrid", "ES"),
    ("瑞典", "SE"), ("🇸🇪", "SE"), ("stockholm", "SE"),
    ("瑞士", "CH"), ("🇨🇭", "CH"), ("zurich", "CH"),
    ("爱尔兰", "IE"), ("🇮🇪", "IE"), ("dublin", "IE"),
    ("葡萄牙", "PT"), ("🇵🇹", "PT"),
    ("罗马尼亚", "RO"), ("🇷🇴", "RO"),
    ("保加利亚", "BG"), ("🇧🇬", "BG"),
    ("哈萨克斯坦", "KZ"), ("🇰🇿", "KZ"),
    ("乌兹别克斯坦", "UZ"), ("🇺🇿", "UZ"),
    ("孟加拉", "BD"), ("🇧🇩", "BD"), ("dhaka", "BD"),
    ("巴基斯坦", "PK"), ("🇵🇰", "PK"), ("lahore", "PK"),
    ("柬埔寨", "KH"), ("🇰🇭", "KH"), ("金边", "KH"),
    ("菲律宾", "PH"), ("🇵🇭", "PH"), ("manila", "PH"),
    ("尼泊尔", "NP"), ("🇳🇵", "NP"),
    ("南非", "ZA"), ("🇿🇦", "ZA"), ("cape town", "ZA"),
    ("尼日利亚", "NG"), ("🇳🇬", "NG"),
    ("肯尼亚", "KE"), ("🇰🇪", "KE"),
    ("埃及", "EG"), ("🇪🇬", "EG"),
    ("摩洛哥", "MA"), ("🇲🇦", "MA"),
    ("冰岛", "IS"), ("🇮🇸", "IS"), ("reykjavik", "IS"),
    ("丹麦", "DK"), ("🇩🇰", "DK"), ("copenhagen", "DK"),
    ("挪威", "NO"), ("🇳🇴", "NO"), ("oslo", "NO"),
    ("芬兰", "FI"), ("🇫🇮", "FI"), ("helsinki", "FI"),
    ("捷克", "CZ"), ("🇨🇿", "CZ"), ("prague", "CZ"),
    ("匈牙利", "HU"), ("🇭🇺", "HU"), ("budapest", "HU"),
    ("希腊", "GR"), ("🇬🇷", "GR"), ("athens", "GR"),
    ("奥地利", "AT"), ("🇦🇹", "AT"), ("vienna", "AT"),
    ("比利时", "BE"), ("🇧🇪", "BE"), ("brussels", "BE"),
    ("立陶宛", "LT"), ("🇱🇹", "LT"), ("vilnius", "LT"), ("🇱🇹", "LT"),
    ("拉脱维亚", "LV"), ("🇱", "LV"), ("riga", "LV"),
    ("爱沙尼亚", "EE"), ("🇪🇪", "EE"), ("tallinn", "EE"),
    ("塞尔维亚", "RS"), ("🇷🇸", "RS"), ("belgrade", "RS"),
    ("克罗地亚", "HR"), ("🇭🇷", "HR"), ("zagreb", "HR"),
    ("斯洛伐克", "SK"), ("🇸🇰", "SK"), ("bratislava", "SK"),
    ("斯洛文尼亚", "SI"), ("🇸🇮", "SI"), ("ljubljana", "SI"),
    ("阿尔巴尼亚", "AL"), ("🇦🇱", "AL"), ("tirana", "AL"),
    ("格鲁吉亚", "GE"), ("🇬🇪", "GE"), ("tbilisi", "GE"),
    ("亚美尼亚", "AM"), ("🇦🇲", "AM"), ("yerevan", "AM"),
    ("阿塞拜疆", "AZ"), ("🇦🇿", "AZ"), ("baku", "AZ"),
    ("摩尔多瓦", "MD"), ("🇲🇩", "MD"), ("chișinău", "MD"), ("chisinau", "MD"),
    ("白俄罗斯", "BY"), ("🇧🇾", "BY"), ("minsk", "BY"),
    ("澳门", "MO"), ("🇲🇴", "MO"), ("macau", "MO"),
    ("中国", "CN"), ("上海", "CN"), ("北京", "CN"), ("广州", "CN"), ("深圳", "CN"),
    ("🇨🇳", "CN"), ("cn-", "CN"),
    ("哥伦比亚", "CO"), ("🇨🇴", "CO"), ("bogota", "CO"),
    ("秘鲁", "PE"), ("🇵🇪", "PE"), ("lima", "PE"),
    ("厄瓜多尔", "EC"), ("🇪🇨", "EC"),
    ("委内瑞拉", "VE"), ("🇻🇪", "VE"),
    ("巴拉圭", "PY"), ("🇵🇾", "PY"),
    ("乌拉圭", "UY"), ("🇺🇾", "UY"),
    ("哥斯达黎加", "CR"), ("🇨🇷", "CR"),
    ("巴拿马", "PA"), ("🇵🇦", "PA"),
    ("多米尼加", "DO"), ("🇩🇴", "DO"),
    ("古巴", "CU"), ("🇨🇺", "CU"),
    ("卢森堡", "LU"), ("🇱🇺", "LU"),
    ("马耳他", "MT"), ("🇲🇹", "MT"),
    ("塞浦路斯", "CY"), ("🇨🇾", "CY"),
    ("波斯尼亚", "BAM"), ("🇧🇦", "BAM"),
    ("北马其顿", "MK"), ("🇲🇰", "MK"), ("skopje", "MK"),
    ("缅甸", "MM"), ("🇲🇲", "MM"), ("yangon", "MM"),
    ("老挝", "LA"), ("🇱🇦", "LA"), ("万象", "LA"), ("vientiane", "LA"),
    ("蒙古", "MN"), ("🇲🇳", "MN"), ("ulaanbaatar", "MN"),
    ("新西兰", "NZ"), ("🇳🇿", "NZ"), ("auckland", "NZ"),
    ("日本", "JP"),
    ("tallinn", "EE"), ("riyadh", "SA"), ("jeddah", "SA"),
    ("tehran", "IR"), ("伊朗", "IR"), ("🇮🇷", "IR"),
    ("baghdad", "IQ"), ("伊拉克", "IQ"),
    ("qatar", "QA"), ("卡塔尔", "QA"), ("🇶", "QA"), ("doha", "QA"),
    ("oman", "OM"), ("阿曼", "OM"), ("muscat", "OM"),
    ("bahrain", "BH"), ("巴林", "BH"),
    ("jordan", "JO"), ("约旦", "JO"), ("amman", "JO"),
    ("lebanon", "LB"), ("黎巴嫩", "LB"), ("beirut", "LB"),
    ("tunisia", "TN"), ("突尼斯", "TN"),
    ("ghana", "GH"), ("加纳", "GH"), ("accra", "GH"),
    ("ethiopia", "ET"), ("埃塞", "ET"), ("addis ababa", "ET"),
    ("tanzania", "TZ"), ("坦桑尼亚", "TZ"),
    ("uganda", "UG"), ("乌干达", "UG"),
    ("zimbabwe", "ZW"), ("津巴布韦", "ZW"),
    ("mozambique", "MZ"), ("莫桑比克", "MZ"),
    ("cameroon", "CM"), ("喀麦隆", "CM"),
    ("senegal", "SN"), ("塞内加尔", "SN"), ("dakar", "SN"),
    ("angola", "AO"), ("安哥拉", "AO"), ("luanda", "AO"),
    ("serbia", "RS"), ("croatia", "HR"), ("bulgaria", "BG"),
    ("estonia", "EE"), ("latvia", "LV"), ("lithuania", "LT"),
    ("slovakia", "SK"), ("slovenia", "SI"), ("albania", "AL"),
    ("georgia", "GE"), ("armenia", "AM"), ("azerbaijan", "AZ"),
    ("moldova", "MD"), ("belarus", "BY"), ("iceland", "IS"),
    ("denmark", "DK"), ("norway", "NO"), ("finland", "FI"),
    ("sweden", "SE"), ("switzerland", "CH"), ("ireland", "IE"),
    ("portugal", "PT"), ("romania", "RO"), ("hungary", "HU"),
    ("greece", "GR"), ("austria", "AT"), ("belgium", "BE"),
    ("luxembourg", "LU"), ("malta", "MT"), ("cyprus", "CY"),
    ("nepal", "NP"), ("katmandu", "NP"), ("kathmandu", "NP"),
    ("srilanka", "LK"), ("科伦坡", "LK"), ("colombo", "LK"),
    ("🇱🇰", "LK"),
    ("🇵🇷", "PR"), ("puerto rico", "PR"),
    ("🇲🇪", "ME"), ("montenegro", "ME"), ("podgorica", "ME"),
    ("🇽🇰", "XK"), ("kosovo", "XK"), ("pristina", "XK"),
    ("🇹🇳", "TN"), ("🇿🇼", "ZW"), ("🇦🇴", "AO"), ("🇨🇲", "CM"),
    ("🇸🇳", "SN"), ("🇺🇬", "UG"), ("🇹🇿", "TZ"), ("🇪🇹", "ET"),
    ("🇬🇭", "GH"), ("🇮🇶", "IQ"), ("🇯🇴", "JO"), ("🇱🇧", "LB"),
    ("🇲🇽", "MX"), ("🇨", "CR"), ("🇦", "PA"), ("🇩🇴", "DO"),
    ("🇨🇺", "CU"), ("🇬🇹", "GT"), ("guatemala", "GT"),
    ("🇭🇳", "HN"), ("honduras", "HN"),
    ("🇸🇻", "SV"), ("el salvador", "SV"),
    ("🇳", "NI"), ("nicaragua", "NI"),
    ("🇧🇴", "BO"), ("bolivia", "BO"), ("la paz", "BO"),
    ("🇵🇾", "PY"), ("asuncion", "PY"),
    ("🇺🇾", "UY"), ("montevideo", "UY"),
    ("🇬🇾", "GY"), ("guyana", "GY"), ("georgetown", "GY"),
    ("🇸🇷", "SR"), ("suriname", "SR"), ("paramaribo", "SR"),
    ("🇯🇲", "JM"), ("jamaica", "JM"), ("kingston", "JM"),
    ("🇹🇹", "TT"), ("trinidad", "TT"),
    ("🇧🇸", "BS"), ("bahamas", "BS"), ("nassau", "BS"),
    ("🇭🇹", "HT"), ("haiti", "HT"), ("port-au-prince", "HT"),
];

impl CountryInfer {
    /// Infer ISO-3166 alpha-2 (or pseudo code) from a node name.
    /// Returns None when no clue found.
    pub fn from_name(name: &str) -> Option<String> {
        let lower = name.to_lowercase();

        // Regional indicator emoji pairs anywhere in the string (e.g. 🇺 -> US)
        let chars: Vec<char> = name.chars().collect();
        let mut i = 0;
        while i + 1 < chars.len() {
            if is_ri(chars[i]) && is_ri(chars[i + 1]) {
                let c1 = ((chars[i] as u32 - 0x1F1E6) as u8 + b'A') as char;
                let c2 = ((chars[i + 1] as u32 - 0x1F1E6) as u8 + b'A') as char;
                return Some(String::from_iter([c1, c2]));
            }
            i += 1;
        }

        // Keyword scan: prefer longest matching keyword to avoid short words matching inside others
        let mut best: Option<(usize, String)> = None;
        for (kw, code) in KEYWORDS {
            if lower.contains(kw) {
                let len = kw.len();
                if best.as_ref().is_none_or(|(bl, _)| len > *bl) {
                    best = Some((len, code.to_string()));
                }
            }
        }
        if let Some((_, c)) = best {
            return Some(c);
        }

        // Prefix scan: e.g. "HK:01", "US-01", "JP_02", "SG01"
        let prefix_chars: Vec<char> = lower.chars().take(3).collect();
        if prefix_chars.len() >= 2 && prefix_chars[0].is_ascii_alphabetic() && prefix_chars[1].is_ascii_alphabetic() {
            let prefix2: String = prefix_chars[..2].iter().collect();
            let next_char = prefix_chars.get(2).copied().unwrap_or(' ');
            if (next_char == ':' || next_char == '-' || next_char == '_' || next_char == ' ' || next_char.is_ascii_digit())
                && matches!(prefix2.as_str(), "hk" | "us" | "jp" | "sg" | "kr" | "tw" | "de" | "gb" | "uk" | "fr" | "nl" | "ca" | "au" | "ru" | "in")
            {
                let code = if prefix2 == "uk" { "GB" } else { prefix2.as_str() }.to_uppercase();
                return Some(code);
            }
        }

        None
    }

    /// Display label for a country code (Chinese-English combined, best effort).
    #[allow(dead_code)]
    pub fn label(code: &str) -> String {
        let flag = code_to_flag(code);
        format!("{} {}", flag, code)
    }
}

pub fn country_name(code: &str) -> &'static str {
    match code.to_uppercase().as_str() {
        "US" => "美国",
        "HK" => "香港",
        "JP" => "日本",
        "SG" => "新加坡",
        "KR" => "韩国",
        "TW" => "台湾",
        "GB" | "UK" => "英国",
        "DE" => "德国",
        "FR" => "法国",
        "NL" => "荷兰",
        "CA" => "加拿大",
        "AU" => "澳大利亚",
        "RU" => "俄罗斯",
        "TR" => "土耳其",
        "BR" => "巴西",
        "IN" => "印度",
        "VN" => "越南",
        "TH" => "泰国",
        "MY" => "马来西亚",
        "ID" => "印尼",
        "AR" => "阿根廷",
        "CL" => "智利",
        "MX" => "墨西哥",
        "AE" => "阿联酋",
        "SA" => "沙特",
        "IL" => "以色列",
        "UA" => "乌克兰",
        "PL" => "波兰",
        "IT" => "意大利",
        "ES" => "西班牙",
        "SE" => "瑞典",
        "CH" => "瑞士",
        "IE" => "爱尔兰",
        "PT" => "葡萄牙",
        "RO" => "罗马尼亚",
        "BG" => "保加利亚",
        "KZ" => "哈萨克斯坦",
        "BD" => "孟加拉",
        "PK" => "巴基斯坦",
        "PH" => "菲律宾",
        "ZA" => "南非",
        "NG" => "尼日利亚",
        "EG" => "埃及",
        "DK" => "丹麦",
        "NO" => "挪威",
        "FI" => "芬兰",
        "CZ" => "捷克",
        "HU" => "匈牙利",
        "AT" => "奥地利",
        "BE" => "比利时",
        "AM" => "亚美尼亚",
        "GE" => "格鲁吉亚",
        "CN" => "中国",
        _ => "其他",
    }
}

fn is_ri(c: char) -> bool {
    matches!(c, '\u{1F1E6}'..='\u{1F1FF}')
}

/// Convert "US" -> "🇺🇸"
pub fn code_to_flag(code: &str) -> String {
    let mut s = String::new();
    for b in code.to_uppercase().chars() {
        if b.is_ascii_uppercase() {
            s.push(char::from_u32(0x1F1E6 + (b as u32 - 'A' as u32)).unwrap_or(' '));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flags() {
        assert_eq!(CountryInfer::from_name("🇺🇸【北美洲】美国04原生丨直连【2x】").as_deref(), Some("US"));
        assert_eq!(CountryInfer::from_name("🇯 免费-日本1-Ver.7").as_deref(), Some("JP"));
        assert_eq!(CountryInfer::from_name("HK-Hong Kong-4282476-bloj").as_deref(), Some("HK"));
        assert_eq!(CountryInfer::from_name("US-San Jose-82005714-xxcu").as_deref(), Some("US"));
    }

    #[test]
    fn test_no_clue() {
        assert_eq!(CountryInfer::from_name("218.33.97.89:62000"), None);
    }

    #[test]
    fn test_flag_render() {
        assert_eq!(code_to_flag("us"), "🇺🇸");
    }
}
