<?php

declare(strict_types=1);

/*
 * User-Agent tables. Order matters (first match wins). Extend freely.
 */
return [
    // In-app browsers: [regex, app name] -> browser "In-app", browser_version "In-app <app>"
    'in_app' => [
        ['~FBAN|FBAV|FB_IAB|FBIOS~', 'Facebook'],
        ['~Instagram~', 'Instagram'],
        ['~WhatsApp~', 'WhatsApp'],
        ['~\bLine/~', 'LINE'],
        ['~Snapchat~', 'Snapchat'],
        ['~musical_ly|BytedanceWebview|TikTok~i', 'TikTok'],
        ['~LinkedInApp~', 'LinkedIn'],
        ['~Twitter(?:Android)?~', 'Twitter'],
        ['~Pinterest~', 'Pinterest'],
        ['~MicroMessenger~', 'WeChat'],
        ['~GSA/~', 'Google App'],
        ['~Telegram~', 'Telegram'],
    ],
    // Browsers: [regex with version capture, name]
    'browsers' => [
        ['~(?:Edg|EdgA|EdgiOS|Edge)/(\d+)~', 'Edge'],
        ['~(?:OPR|OPiOS|OPT)/(\d+)~', 'Opera'],
        ['~Opera Mini/(\d+)~', 'Opera'],
        ['~Opera[/ ](\d+)~', 'Opera'],
        ['~SamsungBrowser/(\d+)~', 'Samsung Internet'],
        ['~YaBrowser/(\d+)~', 'Yandex Browser'],
        ['~UCBrowser/(\d+)~', 'UC Browser'],
        ['~Vivaldi/(\d+)~', 'Vivaldi'],
        ['~(?:Firefox|FxiOS)/(\d+)~', 'Firefox'],
        ['~CriOS/(\d+)~', 'Chrome'],
        ['~(?:Chrome|Chromium)/(\d+)~', 'Chrome'],
        ['~Version/(\d+)[\d.]* (?:Mobile/\S+ )?Safari/~', 'Safari'],
        ['~MSIE (\d+)~', 'IE'],
        ['~Trident/.*rv:(\d+)~', 'IE'],
        ['~AppleWebKit/.*(?:iPhone|iPad|iPod|Macintosh)()~', 'Safari'],
    ],
    // Windows NT version -> label
    'windows' => [
        '10.0' => 'Windows 10/11', '6.3' => 'Windows 8.1', '6.2' => 'Windows 8', '6.1' => 'Windows 7',
        '6.0' => 'Windows Vista', '5.1' => 'Windows XP', '5.2' => 'Windows XP',
    ],
];
