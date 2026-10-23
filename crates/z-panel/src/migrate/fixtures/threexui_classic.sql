BEGIN TRANSACTION;
CREATE TABLE `users` (`id` integer,`username` text,`password` text,PRIMARY KEY (`id`));
CREATE TABLE `inbounds` (`id` integer,`user_id` integer,`up` integer,`down` integer,`total` integer,`all_time` integer DEFAULT 0,`remark` text,`enable` numeric,`expiry_time` integer,`traffic_reset` text DEFAULT "never",`last_traffic_reset_time` integer DEFAULT 0,`listen` text,`port` integer,`protocol` text,`settings` text,`stream_settings` text,`tag` text UNIQUE,`sniffing` text,PRIMARY KEY (`id`));
CREATE TABLE `client_traffics` (`id` integer,`inbound_id` integer,`enable` numeric,`email` text UNIQUE,`up` integer,`down` integer,`all_time` integer,`expiry_time` integer,`total` integer,`reset` integer DEFAULT 0,`last_online` integer DEFAULT 0,PRIMARY KEY (`id`),CONSTRAINT `fk_inbounds_client_stats` FOREIGN KEY (`inbound_id`) REFERENCES `inbounds`(`id`));
CREATE TABLE `settings` (`id` integer,`key` text,`value` text,PRIMARY KEY (`id`));
INSERT INTO `users` (`id`, `username`, `password`) VALUES (1, 'admin', '$2a$10$notarealhash');
INSERT INTO `inbounds` (`id`, `user_id`, `up`, `down`, `total`, `all_time`, `remark`, `enable`, `expiry_time`, `traffic_reset`, `last_traffic_reset_time`, `listen`, `port`, `protocol`, `settings`, `stream_settings`, `tag`, `sniffing`) VALUES (1, 1, 0, 0, 0, 0, 'reality', 1, 0, 'never', 0, '', 443, 'vless', '{
  "clients": [
    {
      "id": "00000001-1111-4222-8333-000000000001",
      "flow": "xtls-rprx-vision",
      "email": "ali@home",
      "limitIp": 2,
      "totalGB": 10737418240,
      "expiryTime": 1803859200000,
      "enable": true,
      "tgId": 12345,
      "subId": "ali0sub1id",
      "comment": "friend",
      "reset": 30,
      "created_at": 1768037400000,
      "updated_at": 1768037400000
    },
    {
      "id": "00000002-1111-4222-8333-000000000002",
      "flow": "xtls-rprx-vision",
      "email": "neg@home",
      "limitIp": 0,
      "totalGB": 0,
      "expiryTime": -2592000000,
      "enable": true,
      "tgId": 0,
      "subId": "negsub",
      "comment": "",
      "reset": 0,
      "created_at": 1768037400000,
      "updated_at": 1768037400000
    },
    {
      "id": "00000003-1111-4222-8333-000000000003",
      "flow": "xtls-rprx-vision",
      "email": "off@home",
      "limitIp": 0,
      "totalGB": 0,
      "expiryTime": 0,
      "enable": false,
      "tgId": 0,
      "subId": "offsub",
      "comment": "",
      "reset": 0,
      "created_at": 1768037400000,
      "updated_at": 1768037400000
    },
    {
      "id": "00000004-1111-4222-8333-000000000004",
      "flow": "xtls-rprx-vision",
      "email": "quota@home",
      "limitIp": 0,
      "totalGB": 1073741824,
      "expiryTime": 0,
      "enable": true,
      "tgId": 0,
      "subId": "quotasub",
      "comment": "",
      "reset": 0,
      "created_at": 1768037400000,
      "updated_at": 1768037400000
    },
    {
      "id": "00000005-1111-4222-8333-000000000005",
      "flow": "xtls-rprx-vision",
      "email": "old@home",
      "limitIp": 0,
      "totalGB": 0,
      "expiryTime": 1700000000000,
      "enable": true,
      "tgId": 0,
      "subId": "oldsub",
      "comment": "",
      "reset": 0,
      "created_at": 1768037400000,
      "updated_at": 1768037400000
    }
  ],
  "decryption": "none",
  "fallbacks": []
}', '{
  "network": "tcp",
  "security": "reality",
  "externalProxy": [],
  "realitySettings": {
    "show": false,
    "xver": 0,
    "dest": "yahoo.com:443",
    "serverNames": [
      "yahoo.com"
    ],
    "privateKey": "uJgMdjW0CJ3x9Eb0QxL2fT8yXoGzA4rVw1pHkNnS5cU",
    "minClient": "",
    "maxClient": "",
    "maxTimediff": 0,
    "shortIds": [
      "ab12cd34"
    ],
    "settings": {
      "publicKey": "Zk3m8vQ1nR6tY0wA9bC2dE4fG7hJ5lN8pS1uV3xX6zA",
      "fingerprint": "chrome",
      "serverName": "",
      "spiderX": "/"
    }
  },
  "tcpSettings": {
    "acceptProxyProtocol": false,
    "header": {
      "type": "none"
    }
  }
}', 'inbound-443', '{
  "enabled": true,
  "destOverride": [
    "http",
    "tls",
    "quic",
    "fakedns"
  ],
  "metadataOnly": false,
  "routeOnly": false
}');
INSERT INTO `inbounds` (`id`, `user_id`, `up`, `down`, `total`, `all_time`, `remark`, `enable`, `expiry_time`, `traffic_reset`, `last_traffic_reset_time`, `listen`, `port`, `protocol`, `settings`, `stream_settings`, `tag`, `sniffing`) VALUES (2, 1, 0, 0, 0, 0, 'vmess-ws', 1, 0, 'never', 0, '', 8443, 'vmess', '{
  "clients": [
    {
      "id": "0000000b-1111-4222-8333-00000000000b",
      "flow": "",
      "email": "ali@home",
      "limitIp": 0,
      "totalGB": 10737418240,
      "expiryTime": 1803859200000,
      "enable": true,
      "tgId": 0,
      "subId": "ali0sub1id",
      "comment": "",
      "reset": 0,
      "created_at": 1768037400000,
      "updated_at": 1768037400000
    },
    {
      "id": "00000006-1111-4222-8333-000000000006",
      "flow": "",
      "email": "bob@home",
      "limitIp": 0,
      "totalGB": 0,
      "expiryTime": 0,
      "enable": true,
      "tgId": 0,
      "subId": "bobsub",
      "comment": "",
      "reset": 0,
      "created_at": 1768037400000,
      "updated_at": 1768037400000
    },
    {
      "id": "00000007-1111-4222-8333-000000000007",
      "flow": "",
      "email": "shared@home",
      "limitIp": 0,
      "totalGB": 0,
      "expiryTime": 0,
      "enable": true,
      "tgId": 0,
      "subId": "bobsub",
      "comment": "",
      "reset": 0,
      "created_at": 1768037400000,
      "updated_at": 1768037400000
    }
  ]
}', '{
  "network": "ws",
  "security": "none",
  "externalProxy": [],
  "wsSettings": {
    "acceptProxyProtocol": false,
    "path": "/ws",
    "host": "",
    "headers": {}
  }
}', 'inbound-8443', '{
  "enabled": true,
  "destOverride": [
    "http",
    "tls",
    "quic",
    "fakedns"
  ],
  "metadataOnly": false,
  "routeOnly": false
}');
INSERT INTO `inbounds` (`id`, `user_id`, `up`, `down`, `total`, `all_time`, `remark`, `enable`, `expiry_time`, `traffic_reset`, `last_traffic_reset_time`, `listen`, `port`, `protocol`, `settings`, `stream_settings`, `tag`, `sniffing`) VALUES (3, 1, 0, 0, 0, 0, 'trojan-tls', 1, 0, 'never', 0, '', 2096, 'trojan', '{
  "clients": [
    {
      "password": "trojan-secret-1",
      "flow": "",
      "email": "troj@home",
      "limitIp": 0,
      "totalGB": 0,
      "expiryTime": 0,
      "enable": true,
      "tgId": 0,
      "subId": "trojsub",
      "comment": "",
      "reset": 0
    }
  ],
  "fallbacks": []
}', '{
  "network": "tcp",
  "security": "tls",
  "externalProxy": [],
  "tlsSettings": {
    "serverName": "trojan.example.com",
    "minVersion": "1.2",
    "maxVersion": "1.3",
    "cipherSuites": "",
    "rejectUnknownSni": false,
    "disableSystemRoot": false,
    "enableSessionResumption": false,
    "certificates": [
      {
        "certificateFile": "",
        "keyFile": "",
        "certificate": [
          "-----BEGIN CERTIFICATE-----",
          "MIIBrTCCAVSgAwIBAgIUVGQbjUmSLtCHsVaxb36sbYBxDhgwCgYIKoZIzj0EAwIw",
          "HTEbMBkGA1UEAwwSdHJvamFuLmV4YW1wbGUuY29tMB4XDTI2MTAyMzEzMTQyMVoX",
          "DTM2MTAyMDEzMTQyMVowHTEbMBkGA1UEAwwSdHJvamFuLmV4YW1wbGUuY29tMFkw",
          "EwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEHYRdU1mJqhbFn6zJEcr4BrfqbzIRbCAD",
          "fnd6RbDDODxQO6qlXWdu3zos+Mose8qW+U3tbzVEbEgcpJk5kFUFAaNyMHAwHQYD",
          "VR0OBBYEFAT5L9hfVq+mAD4988WN7JV5MpxNMB8GA1UdIwQYMBaAFAT5L9hfVq+m",
          "AD4988WN7JV5MpxNMA8GA1UdEwEB/wQFMAMBAf8wHQYDVR0RBBYwFIISdHJvamFu",
          "LmV4YW1wbGUuY29tMAoGCCqGSM49BAMCA0cAMEQCIBkleU1XbmWyTmEMN3YN9QID",
          "UbX3xoz543hSgc9n+IgqAiAoH6Hdzsnzsvyysc82/eXPoeeCzAcc47V69bv6AFks",
          "cA==",
          "-----END CERTIFICATE-----"
        ],
        "key": [
          "-----BEGIN PRIVATE KEY-----",
          "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgaFy9pdEWpV3GrMGb",
          "1bhS+kQeCEG3TwmpftA+/K7gMyqhRANCAAQdhF1TWYmqFsWfrMkRyvgGt+pvMhFs",
          "IAN+d3pFsMM4PFA7qqVdZ27fOiz4yix7ypb5Te1vNURsSBykmTmQVQUB",
          "-----END PRIVATE KEY-----"
        ],
        "oneTimeLoading": false,
        "usage": "encipherment",
        "buildChain": false
      }
    ],
    "alpn": [
      "h2",
      "http/1.1"
    ],
    "settings": {
      "allowInsecure": false,
      "fingerprint": "chrome"
    }
  },
  "tcpSettings": {
    "acceptProxyProtocol": false,
    "header": {
      "type": "none"
    }
  }
}', 'inbound-2096', '{
  "enabled": true,
  "destOverride": [
    "http",
    "tls",
    "quic",
    "fakedns"
  ],
  "metadataOnly": false,
  "routeOnly": false
}');
INSERT INTO `inbounds` (`id`, `user_id`, `up`, `down`, `total`, `all_time`, `remark`, `enable`, `expiry_time`, `traffic_reset`, `last_traffic_reset_time`, `listen`, `port`, `protocol`, `settings`, `stream_settings`, `tag`, `sniffing`) VALUES (4, 1, 0, 0, 0, 0, 'hysteria', 1, 0, 'never', 0, '', 4443, 'hysteria', '{
  "version": 2,
  "clients": [
    {
      "auth": "hy-auth-1",
      "email": "hy@home",
      "limitIp": 0,
      "totalGB": 0,
      "expiryTime": 0,
      "enable": true,
      "tgId": 0,
      "subId": "hysub",
      "comment": "",
      "reset": 0
    }
  ]
}', '{
  "network": "hysteria",
  "security": "tls"
}', 'inbound-4443', '{
  "enabled": true,
  "destOverride": [
    "http",
    "tls",
    "quic",
    "fakedns"
  ],
  "metadataOnly": false,
  "routeOnly": false
}');
INSERT INTO `inbounds` (`id`, `user_id`, `up`, `down`, `total`, `all_time`, `remark`, `enable`, `expiry_time`, `traffic_reset`, `last_traffic_reset_time`, `listen`, `port`, `protocol`, `settings`, `stream_settings`, `tag`, `sniffing`) VALUES (5, 1, 0, 0, 0, 0, 'ss', 1, 0, 'never', 0, '', 1080, 'shadowsocks', '{
  "method": "chacha20-ietf-poly1305",
  "password": "server-pass",
  "network": "tcp,udp",
  "clients": [
    {
      "method": "",
      "password": "ss-client-pass",
      "email": "ss@home",
      "limitIp": 0,
      "totalGB": 0,
      "expiryTime": 0,
      "enable": true,
      "tgId": 0,
      "subId": "sssub",
      "comment": "",
      "reset": 0
    }
  ]
}', '{
  "network": "tcp",
  "security": "none"
}', 'inbound-1080', '{
  "enabled": true,
  "destOverride": [
    "http",
    "tls",
    "quic",
    "fakedns"
  ],
  "metadataOnly": false,
  "routeOnly": false
}');
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (1, 1, 1, 'ali@home', 1073741824, 2147483648, 3221225472, 1803859200000, 10737418240, 30, 1792692300000);
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (2, 1, 1, 'neg@home', 0, 0, 0, -2592000000, 0, 0, 0);
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (3, 1, 0, 'off@home', 5, 5, 10, 0, 0, 0, 0);
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (4, 1, 1, 'quota@home', 1073741824, 214748364, 1288490188, 0, 1073741824, 0, 0);
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (5, 1, 1, 'old@home', 100, 200, 300, 1700000000000, 0, 0, 0);
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (6, 2, 1, 'bob@home', 7, 8, 15, 0, 0, 0, 0);
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (7, 2, 1, 'shared@home', 0, 0, 0, 0, 0, 0, 0);
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (8, 3, 1, 'troj@home', 1000, 2000, 3000, 0, 0, 0, 0);
INSERT INTO `client_traffics` (`id`, `inbound_id`, `enable`, `email`, `up`, `down`, `all_time`, `expiry_time`, `total`, `reset`, `last_online`) VALUES (9, 5, 1, 'ss@home', 0, 0, 0, 0, 0, 0, 0);
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (1, 'subPath', '/sub/');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (2, 'subJsonPath', '/json/');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (3, 'subClashPath', '/clash/');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (4, 'subTitle', 'My VPN');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (5, 'subUpdates', '6');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (6, 'subSupportUrl', 'https://t.me/support');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (7, 'subJsonEnable', 'true');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (8, 'subClashEnable', 'false');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (9, 'webPort', '2053');
INSERT INTO `settings` (`id`, `key`, `value`) VALUES (10, 'subPort', '2096');
COMMIT;
