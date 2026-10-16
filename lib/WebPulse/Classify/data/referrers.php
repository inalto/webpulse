<?php

declare(strict_types=1);

/*
 * Referrer classification tables. Hosts are matched without a leading
 * "www.". Patterns are PCRE fragments anchored by the classifier
 * (`~^(?:pattern)$~`). Extend freely.
 */
return [
    // [host pattern, engine key, query parameters carrying the keyword]
    'search' => [
        ['(?:[a-z0-9-]+\.)?google\.(?:[a-z]{2,3}|co\.[a-z]{2}|com\.[a-z]{2})', 'google', ['q']],
        ['com\.google\.android\.googlequicksearchbox', 'google', []],
        ['com\.google\.android\.gm\.?lite', 'google', []],
        ['(?:[a-z]{2}\.)?bing\.com', 'bing', ['q']],
        ['(?:html\.|lite\.)?duckduckgo\.com', 'duckduckgo', ['q']],
        ['(?:[a-z]{2}\.)?search\.yahoo\.(?:com|co\.jp|[a-z]{2})', 'yahoo', ['p', 'q']],
        ['search\.yahoo\.co\.jp', 'yahoo', ['p']],
        ['yandex\.(?:ru|com|by|kz|ua|com\.tr|uz)', 'yandex', ['text']],
        ['ya\.ru', 'yandex', ['text']],
        ['(?:m\.)?baidu\.com', 'baidu', ['wd', 'word']],
        ['ecosia\.org', 'ecosia', ['q']],
        ['(?:[a-z]+\.)?qwant\.com', 'qwant', ['q']],
        ['startpage\.com', 'startpage', ['query', 'q']],
        ['(?:[a-z]+\.)?startpage\.com', 'startpage', ['query', 'q']],
        ['search\.brave\.com', 'brave', ['q']],
        ['(?:search\.|m\.search\.)?naver\.com', 'naver', ['query']],
        ['(?:search\.)?seznam\.cz', 'seznam', ['q']],
        ['(?:[a-z]+\.)?ask\.com', 'ask', ['q']],
        ['search\.aol\.(?:com|co\.uk)', 'aol', ['q', 'query']],
        ['(?:[a-z]+\.)?yahoo\.com', 'yahoo', ['p']],
    ],
    // [host pattern, network name]
    'social' => [
        ['(?:[a-z0-9-]+\.)?facebook\.com', 'facebook'],
        ['fb\.me', 'facebook'],
        ['(?:[a-z0-9-]+\.)?fb\.com', 'facebook'],
        ['(?:l\.)?instagram\.com', 'instagram'],
        ['(?:mobile\.)?twitter\.com', 'twitter'],
        ['x\.com', 'twitter'],
        ['t\.co', 'twitter'],
        ['(?:[a-z]{2}\.)?linkedin\.com', 'linkedin'],
        ['lnkd\.in', 'linkedin'],
        ['(?:[a-z]{2}\.)?pinterest\.(?:com|[a-z]{2}|co\.uk|com\.au)', 'pinterest'],
        ['pin\.it', 'pinterest'],
        ['(?:old\.|new\.|out\.|m\.)?reddit\.com', 'reddit'],
        ['(?:[a-z]+\.)?tiktok\.com', 'tiktok'],
        ['(?:m\.)?youtube\.com', 'youtube'],
        ['youtu\.be', 'youtube'],
        ['threads\.(?:net|com)', 'threads'],
        ['bsky\.app', 'bluesky'],
        ['(?:[a-z]+\.)?snapchat\.com', 'snapchat'],
        ['(?:web\.|api\.|chat\.)?whatsapp\.com', 'whatsapp'],
        ['wa\.me', 'whatsapp'],
        ['(?:web\.)?telegram\.org', 'telegram'],
        ['t\.me', 'telegram'],
        ['mastodon\.(?:social|online|world)', 'mastodon'],
        ['(?:mstdn\.social|fosstodon\.org|hachyderm\.io|infosec\.exchange|mas\.to|techhub\.social|mastodon\.art)', 'mastodon'],
        ['(?:m\.)?vk\.com', 'vk'],
        ['ok\.ru', 'odnoklassniki'],
        ['(?:[a-z0-9-]+\.)?tumblr\.com', 'tumblr'],
        ['(?:[a-z]{2}\.)?quora\.com', 'quora'],
        ['news\.ycombinator\.com', 'hacker news'],
        ['(?:[a-z]+\.)?discord(?:app)?\.com', 'discord'],
        ['(?:[a-z]+\.)?medium\.com', 'medium'],
        ['org\.telegram\.messenger', 'telegram'],
        ['com\.facebook\.katana', 'facebook'],
        ['com\.instagram\.android', 'instagram'],
        ['com\.twitter\.android', 'twitter'],
        ['com\.linkedin\.android', 'linkedin'],
        ['com\.reddit\.frontpage', 'reddit'],
        ['com\.whatsapp', 'whatsapp'],
        ['com\.zhiliaoapp\.musically', 'tiktok'],
    ],
    // webmail hosts (channel "email")
    'email' => [
        'mail\.google\.com', 'com\.google\.android\.gm', 'outlook\.live\.com', 'outlook\.office(?:365)?\.com',
        '(?:[a-z0-9-]+\.)?mail\.yahoo\.com', 'mail\.aol\.com', 'mail\.proton\.me', 'mail\.protonmail\.com',
        'mail\.zoho\.(?:com|eu)', 'mail\.yandex\.(?:ru|com)', 'e\.mail\.ru', 'mail\.libero\.it', 'mail\.virgilio\.it',
        'webmail\.[a-z0-9.-]+', 'mail\.tiscali\.it', 'posteo\.de', 'mail\.gmx\.(?:net|com|de)', 'navigator\.gmx\.(?:net|com)',
        'web\.de', 'email\.t-online\.de', 'mail\.orange\.fr', 'webmail\.orange\.fr', 'icloud\.com', 'fastmail\.com',
        'app\.fastmail\.com', 'mail\.163\.com', 'mail\.qq\.com', 'outlook\.com',
    ],
    // utm_medium values meaning a paid click
    'paid_mediums' => ['cpc', 'ppc', 'paid', 'paidsearch', 'paid-search', 'paid_search', 'display', 'cpm', 'banner', 'cpv', 'cpa', 'paidsocial', 'paid-social', 'paid_social'],
    'email_mediums' => ['email', 'e-mail', 'e_mail', 'newsletter', 'mail'],
    'social_mediums' => ['social', 'social-network', 'social-media', 'social_media', 'sm', 'social network', 'social media'],
    'organic_mediums' => ['organic'],
    // query parameters that identify an ad click
    'click_ids' => ['gclid', 'fbclid', 'msclkid', 'gbraid', 'wbraid', 'dclid', 'ttclid', 'twclid', 'li_fat_id', 'yclid'],
];
