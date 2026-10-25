PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE admin_notification_reminders (
	admin_id INTEGER NOT NULL, 
	type VARCHAR(15) NOT NULL, 
	threshold INTEGER DEFAULT '0' NOT NULL, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_admin_notification_reminders PRIMARY KEY (id), 
	CONSTRAINT uq_admin_notification_reminders_admin_id_type_threshold UNIQUE (admin_id, type, threshold), 
	CONSTRAINT fk_admin_notification_reminders_admin_id_admins FOREIGN KEY(admin_id) REFERENCES admins (id) ON DELETE CASCADE
);
CREATE TABLE admin_roles (
	name VARCHAR(64) NOT NULL, 
	is_owner BOOLEAN DEFAULT '0' NOT NULL, 
	permissions JSON NOT NULL, 
	limits JSON NOT NULL, 
	features JSON NOT NULL, 
	access JSON NOT NULL, 
	hwid JSON NOT NULL, 
	disabled_when_limited BOOLEAN DEFAULT '0' NOT NULL, 
	disconnect_users_when_limited BOOLEAN DEFAULT '1' NOT NULL, 
	disconnect_users_when_disabled BOOLEAN DEFAULT '1' NOT NULL, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_admin_roles PRIMARY KEY (id), 
	CONSTRAINT uq_admin_roles_name UNIQUE (name)
);
INSERT INTO "admin_roles" VALUES('owner',1,'{}','{}','{}','{}','{}',0,1,1,'2026-10-25 14:10:25.916071',1);
INSERT INTO "admin_roles" VALUES('administrator',0,'{}','{}','{}','{}','{}',0,1,1,'2026-10-25 14:10:25.916107',2);
INSERT INTO "admin_roles" VALUES('operator',0,'{}','{}','{}','{}','{}',0,1,1,'2026-10-25 14:10:25.916120',3);
CREATE TABLE admin_usage_logs (
	admin_id INTEGER NOT NULL, 
	used_traffic_at_reset BIGINT NOT NULL, 
	reset_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_admin_usage_logs PRIMARY KEY (id), 
	CONSTRAINT fk_admin_usage_logs_admin_id_admins FOREIGN KEY(admin_id) REFERENCES admins (id)
);
CREATE TABLE admins (
	username VARCHAR(34) NOT NULL, 
	hashed_password VARCHAR(128) NOT NULL, 
	password_reset_at DATETIME, 
	telegram_id BIGINT, 
	discord_webhook VARCHAR(1024), 
	used_traffic BIGINT NOT NULL, 
	data_limit BIGINT, 
	status VARCHAR(8) DEFAULT 'active' NOT NULL, 
	last_status_change DATETIME, 
	sub_template VARCHAR(1024), 
	sub_domain VARCHAR(256), 
	profile_title VARCHAR(512), 
	support_url VARCHAR(1024), 
	custom_variables JSON, 
	notification_enable JSON, 
	note VARCHAR(500), 
	role_id INTEGER NOT NULL, 
	permission_overrides JSON, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_admins PRIMARY KEY (id), 
	CONSTRAINT ck_admins_adminstatus CHECK (status IN ('active', 'disabled', 'limited')), 
	CONSTRAINT fk_admins_role_id_admin_roles FOREIGN KEY(role_id) REFERENCES admin_roles (id)
);
INSERT INTO "admins" VALUES('root','$2b$12$notarealhashnotarealhashnotarealhashnotarealhash',NULL,NULL,NULL,0,NULL,'active',NULL,NULL,NULL,NULL,NULL,'null','null',NULL,1,'null','2026-10-25 14:10:25.917166',1);
INSERT INTO "admins" VALUES('shop','$2b$12$notarealhashnotarealhashnotarealhashnotarealhash',NULL,NULL,NULL,0,NULL,'active',NULL,NULL,NULL,NULL,NULL,'null','null',NULL,3,'null','2026-10-25 14:10:25.917329',2);
CREATE TABLE api_keys (
	admin_id INTEGER NOT NULL, 
	name VARCHAR(128) NOT NULL, 
	key_hash VARCHAR(128) NOT NULL, 
	api_key_trimmed VARCHAR(16) NOT NULL, 
	permissions JSON NOT NULL, 
	inherit_permissions BOOLEAN DEFAULT '1' NOT NULL, 
	note VARCHAR(512), 
	expire_date DATETIME, 
	revoked_at DATETIME, 
	status VARCHAR(8) DEFAULT 'active' NOT NULL, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_api_keys PRIMARY KEY (id), 
	CONSTRAINT uq_api_keys_key_hash UNIQUE (key_hash), 
	CONSTRAINT uq_api_keys_admin_id UNIQUE (admin_id, name), 
	CONSTRAINT fk_api_keys_admin_id_admins FOREIGN KEY(admin_id) REFERENCES admins (id) ON DELETE CASCADE, 
	CONSTRAINT ck_api_keys_apikeystatus CHECK (status IN ('active', 'disabled'))
);
CREATE TABLE client_templates (
	name VARCHAR(64) NOT NULL, 
	template_type VARCHAR(32) NOT NULL, 
	content TEXT NOT NULL, 
	is_default BOOLEAN DEFAULT '0' NOT NULL, 
	is_system BOOLEAN DEFAULT '0' NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_client_templates PRIMARY KEY (id), 
	CONSTRAINT uq_client_templates_template_type UNIQUE (template_type, name)
);
CREATE TABLE core_configs (
	name VARCHAR(256) NOT NULL, 
	config JSON NOT NULL, 
	type VARCHAR(7) DEFAULT 'xray' NOT NULL, 
	exclude_inbound_tags VARCHAR(2048), 
	fallbacks_inbound_tags VARCHAR(2048), 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_core_configs PRIMARY KEY (id)
);
INSERT INTO "core_configs" VALUES('Default','{"log": {"loglevel": "warning"}, "inbounds": [{"tag": "VLESS TCP REALITY", "listen": "0.0.0.0", "port": 443, "protocol": "vless", "settings": {"clients": [], "decryption": "none"}, "streamSettings": {"network": "tcp", "tcpSettings": {}, "security": "reality", "realitySettings": {"show": false, "dest": "yahoo.com:443", "xver": 0, "serverNames": ["yahoo.com", "www.yahoo.com"], "privateKey": "bSXPc0xJod0nPk2Pq19b240QmewF6P3Hwdc0d3ZIq3M", "shortIds": ["ab12cd34"]}}, "sniffing": {"enabled": true, "destOverride": ["http", "tls", "quic"]}}, {"tag": "Trojan WS", "listen": "0.0.0.0", "port": 8443, "protocol": "trojan", "settings": {"clients": []}, "streamSettings": {"network": "ws", "wsSettings": {"path": "/trojan"}, "security": "none"}}, {"tag": "VMess WS", "listen": "0.0.0.0", "port": 8080, "protocol": "vmess", "settings": {"clients": []}, "streamSettings": {"network": "ws", "wsSettings": {"path": "/vmess"}, "security": "none"}}, {"tag": "Shadowsocks TCP", "listen": "0.0.0.0", "port": 1080, "protocol": "shadowsocks", "settings": {"clients": [], "network": "tcp,udp"}}, {"tag": "API_INBOUND", "listen": "127.0.0.1", "port": 62789, "protocol": "dokodemo-door", "settings": {"address": "127.0.0.1"}}], "outbounds": [{"protocol": "freedom", "tag": "DIRECT"}, {"protocol": "blackhole", "tag": "BLOCK"}], "routing": {"rules": [{"inboundTag": ["API_INBOUND"], "outboundTag": "API", "type": "field"}]}}','xray','API_INBOUND','','2026-10-25 14:10:25.912371',1);
CREATE TABLE groups (
	name VARCHAR(64) NOT NULL, 
	is_disabled BOOLEAN DEFAULT '0' NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_groups PRIMARY KEY (id)
);
INSERT INTO "groups" VALUES('Premium',0,1);
INSERT INTO "groups" VALUES('Basic',0,2);
INSERT INTO "groups" VALUES('Retired',1,3);
CREATE TABLE hosts (
	remark VARCHAR(256) NOT NULL, 
	port INTEGER, 
	path VARCHAR(256), 
	priority INTEGER NOT NULL, 
	allowinsecure BOOLEAN, 
	address VARCHAR(256) NOT NULL, 
	sni VARCHAR(1000), 
	host VARCHAR(1000), 
	inbound_tag VARCHAR(256), 
	security VARCHAR(15) NOT NULL, 
	alpn VARCHAR(14), 
	fingerprint VARCHAR(16) DEFAULT 'none' NOT NULL, 
	is_disabled BOOLEAN, 
	fragment_settings JSON, 
	noise_settings JSON, 
	random_user_agent BOOLEAN DEFAULT '0' NOT NULL, 
	use_sni_as_host BOOLEAN DEFAULT '0' NOT NULL, 
	http_headers JSON, 
	transport_settings JSON, 
	mux_settings JSON, 
	status VARCHAR(60) DEFAULT '', 
	ech JSON, 
	vless_route VARCHAR(4), 
	pinned_peer_cert_sha256 VARCHAR(128), 
	verify_peer_cert_by_name VARCHAR(1000), 
	wireguard_overrides JSON, 
	subscription_templates JSON, 
	final_mask_settings JSON, 
	cipher_suites VARCHAR(1024), 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_hosts PRIMARY KEY (id), 
	CONSTRAINT fk_hosts_inbound_tag_inbounds FOREIGN KEY(inbound_tag) REFERENCES inbounds (tag) ON DELETE SET NULL ON UPDATE CASCADE
);
INSERT INTO "hosts" VALUES('{USERNAME} - {DATA_LEFT}',2053,'/cdn',1,0,'cdn.example.com','cdn.example.com','cdn.example.com','Trojan WS','tls','h2,http/1.1','chrome',0,NULL,NULL,0,0,NULL,NULL,'{"enabled": true}','',NULL,NULL,NULL,'',NULL,NULL,NULL,NULL,1);
INSERT INTO "hosts" VALUES('reality direct',NULL,NULL,2,NULL,'direct.example.com','','','VLESS TCP REALITY','inbound_default','','none',1,NULL,NULL,0,0,NULL,NULL,NULL,'',NULL,NULL,NULL,'',NULL,NULL,NULL,NULL,2);
CREATE TABLE inbounds (
	tag VARCHAR(256) NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_inbounds PRIMARY KEY (id)
);
INSERT INTO "inbounds" VALUES('VLESS TCP REALITY',1);
INSERT INTO "inbounds" VALUES('Trojan WS',2);
INSERT INTO "inbounds" VALUES('VMess WS',3);
INSERT INTO "inbounds" VALUES('Shadowsocks TCP',4);
CREATE TABLE inbounds_groups_association (
	inbound_id INTEGER NOT NULL, 
	group_id INTEGER NOT NULL, 
	CONSTRAINT pk_inbounds_groups_association PRIMARY KEY (inbound_id, group_id), 
	CONSTRAINT fk_inbounds_groups_association_inbound_id_inbounds FOREIGN KEY(inbound_id) REFERENCES inbounds (id), 
	CONSTRAINT fk_inbounds_groups_association_group_id_groups FOREIGN KEY(group_id) REFERENCES groups (id)
);
INSERT INTO "inbounds_groups_association" VALUES(3,3);
INSERT INTO "inbounds_groups_association" VALUES(1,1);
INSERT INTO "inbounds_groups_association" VALUES(2,1);
INSERT INTO "inbounds_groups_association" VALUES(3,2);
INSERT INTO "inbounds_groups_association" VALUES(4,2);
CREATE TABLE jwt (
	secret_key VARCHAR(64) NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_jwt PRIMARY KEY (id)
);
INSERT INTO "jwt" VALUES('5f3a9c1e7b2d4f60a8c3e1d9b7f50426c8a1e3d5f7092b4c6e8a0d2f4b6c8e1a',1);
CREATE TABLE next_plans (
	user_id INTEGER NOT NULL, 
	user_template_id INTEGER, 
	data_limit BIGINT NOT NULL, 
	expire INTEGER, 
	add_remaining_traffic BOOLEAN DEFAULT '0' NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_next_plans PRIMARY KEY (id), 
	CONSTRAINT fk_next_plans_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE, 
	CONSTRAINT fk_next_plans_user_template_id_user_templates FOREIGN KEY(user_template_id) REFERENCES user_templates (id) ON DELETE SET NULL
);
INSERT INTO "next_plans" VALUES(6,NULL,53687091200,2592000,1,1);
CREATE TABLE node_stats (
	node_id INTEGER NOT NULL, 
	mem_total BIGINT NOT NULL, 
	mem_used BIGINT NOT NULL, 
	cpu_cores INTEGER NOT NULL, 
	cpu_usage FLOAT NOT NULL, 
	incoming_bandwidth_speed BIGINT NOT NULL, 
	outgoing_bandwidth_speed BIGINT NOT NULL, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_node_stats PRIMARY KEY (id), 
	CONSTRAINT fk_node_stats_node_id_nodes FOREIGN KEY(node_id) REFERENCES nodes (id)
);
CREATE TABLE node_usage_reset_logs (
	node_id INTEGER NOT NULL, 
	uplink BIGINT NOT NULL, 
	downlink BIGINT NOT NULL, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_node_usage_reset_logs PRIMARY KEY (id), 
	CONSTRAINT fk_node_usage_reset_logs_node_id_nodes FOREIGN KEY(node_id) REFERENCES nodes (id) ON DELETE CASCADE
);
CREATE TABLE node_usages (
	created_at DATETIME NOT NULL, 
	node_id INTEGER, 
	uplink BIGINT NOT NULL, 
	downlink BIGINT NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_node_usages PRIMARY KEY (id), 
	CONSTRAINT uq_node_usages_created_at UNIQUE (created_at, node_id), 
	CONSTRAINT fk_node_usages_node_id_nodes FOREIGN KEY(node_id) REFERENCES nodes (id) ON DELETE CASCADE
);
CREATE TABLE node_user_usages (
	created_at DATETIME NOT NULL, 
	user_id INTEGER NOT NULL, 
	node_id INTEGER, 
	used_traffic BIGINT NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_node_user_usages PRIMARY KEY (id), 
	CONSTRAINT uq_node_user_usages_created_at UNIQUE (created_at, user_id, node_id), 
	CONSTRAINT fk_node_user_usages_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE, 
	CONSTRAINT fk_node_user_usages_node_id_nodes FOREIGN KEY(node_id) REFERENCES nodes (id) ON DELETE CASCADE
);
INSERT INTO "node_user_usages" VALUES('2026-10-22 05:00:00.000000',1,NULL,1073741824,1);
INSERT INTO "node_user_usages" VALUES('2026-10-22 06:00:00.000000',1,NULL,536870912,2);
INSERT INTO "node_user_usages" VALUES('2026-10-22 05:00:00.000000',1,1,268435456,3);
INSERT INTO "node_user_usages" VALUES('2026-10-22 06:00:00.000000',1,1,134217728,4);
INSERT INTO "node_user_usages" VALUES('2026-10-21 23:00:00.000000',3,NULL,1073741824,5);
CREATE TABLE nodes (
	name VARCHAR(256) COLLATE BINARY NOT NULL, 
	address VARCHAR(256) NOT NULL, 
	port INTEGER NOT NULL, 
	api_port INTEGER DEFAULT '62051' NOT NULL, 
	xray_version VARCHAR(32), 
	message VARCHAR(1024), 
	server_ca VARCHAR(2048) NOT NULL, 
	api_key VARCHAR(36), 
	node_version VARCHAR(32), 
	core_config_id INTEGER, 
	status VARCHAR(10) NOT NULL, 
	last_status_change DATETIME, 
	uplink BIGINT NOT NULL, 
	downlink BIGINT NOT NULL, 
	data_limit BIGINT NOT NULL, 
	data_limit_reset_strategy VARCHAR(8) NOT NULL, 
	reset_time INTEGER DEFAULT (-1) NOT NULL, 
	usage_coefficient FLOAT DEFAULT (1.0) NOT NULL, 
	connection_type VARCHAR(4) DEFAULT 'grpc' NOT NULL, 
	keep_alive INTEGER NOT NULL, 
	default_timeout INTEGER DEFAULT 10 NOT NULL, 
	internal_timeout INTEGER DEFAULT 15 NOT NULL, 
	proxy_url VARCHAR(256), 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_nodes PRIMARY KEY (id), 
	CONSTRAINT uq_nodes_name UNIQUE (name), 
	CONSTRAINT fk_nodes_core_config_id_core_configs FOREIGN KEY(core_config_id) REFERENCES core_configs (id) ON DELETE SET NULL
);
CREATE TABLE notification_reminders (
	user_id INTEGER NOT NULL, 
	type VARCHAR(15) NOT NULL, 
	threshold INTEGER, 
	expires_at DATETIME, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_notification_reminders PRIMARY KEY (id), 
	CONSTRAINT fk_notification_reminders_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);
CREATE TABLE settings (
	telegram JSON NOT NULL, 
	webhook JSON NOT NULL, 
	notification_settings JSON NOT NULL, 
	notification_enable JSON NOT NULL, 
	subscription JSON NOT NULL, 
	hwid JSON NOT NULL, 
	general JSON NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_settings PRIMARY KEY (id)
);
CREATE TABLE system (
	uplink BIGINT NOT NULL, 
	downlink BIGINT NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_system PRIMARY KEY (id)
);
CREATE TABLE temp_keys (
	"key" VARCHAR(36) NOT NULL, 
	action VARCHAR(32) NOT NULL, 
	expires_at DATETIME NOT NULL, 
	used_at DATETIME, 
	used_by_ip VARCHAR(45), 
	CONSTRAINT pk_temp_keys PRIMARY KEY ("key")
);
CREATE TABLE template_group_association (
	user_template_id INTEGER, 
	group_id INTEGER, 
	CONSTRAINT fk_template_group_association_user_template_id_user_templates FOREIGN KEY(user_template_id) REFERENCES user_templates (id), 
	CONSTRAINT fk_template_group_association_group_id_groups FOREIGN KEY(group_id) REFERENCES groups (id)
);
INSERT INTO "template_group_association" VALUES(1,1);
CREATE TABLE user_hwids (
	user_id INTEGER NOT NULL, 
	hwid VARCHAR(256) NOT NULL, 
	device_os VARCHAR(256), 
	os_version VARCHAR(128), 
	device_model VARCHAR(256), 
	last_used_at DATETIME NOT NULL, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_user_hwids PRIMARY KEY (id), 
	CONSTRAINT uq_user_hwids_user_id UNIQUE (user_id, hwid), 
	CONSTRAINT fk_user_hwids_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);
CREATE TABLE user_subscription_updates (
	user_id INTEGER NOT NULL, 
	user_agent VARCHAR(512) NOT NULL, 
	ip VARCHAR(64), 
	hwid VARCHAR(256), 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_user_subscription_updates PRIMARY KEY (id), 
	CONSTRAINT fk_user_subscription_updates_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);
CREATE TABLE user_templates (
	name VARCHAR(64) NOT NULL, 
	username_prefix VARCHAR(20), 
	username_suffix VARCHAR(20), 
	extra_settings JSON, 
	data_limit BIGINT NOT NULL, 
	hwid_limit BIGINT, 
	expire_duration BIGINT NOT NULL, 
	on_hold_timeout INTEGER, 
	status VARCHAR(7) NOT NULL, 
	reset_usages BOOLEAN DEFAULT '0' NOT NULL, 
	data_limit_reset_strategy VARCHAR(8) DEFAULT 'no_reset' NOT NULL, 
	is_disabled BOOLEAN DEFAULT '0' NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_user_templates PRIMARY KEY (id), 
	CONSTRAINT uq_user_templates_name UNIQUE (name)
);
INSERT INTO "user_templates" VALUES('monthly-10g',NULL,NULL,NULL,10737418240,NULL,2592000,NULL,'active',0,'no_reset',0,1);
CREATE TABLE user_usage_logs (
	user_id INTEGER, 
	used_traffic_at_reset BIGINT NOT NULL, 
	reset_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_user_usage_logs PRIMARY KEY (id), 
	CONSTRAINT fk_user_usage_logs_user_id_users FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);
INSERT INTO "user_usage_logs" VALUES(6,5368709120,'2026-10-25 14:10:25.927478',1);
CREATE TABLE users (
	username VARCHAR(128) COLLATE BINARY NOT NULL, 
	proxy_settings JSON DEFAULT '{}' NOT NULL, 
	status VARCHAR(8) NOT NULL, 
	used_traffic BIGINT NOT NULL, 
	data_limit BIGINT, 
	data_limit_reset_strategy VARCHAR(8) NOT NULL, 
	expire DATETIME, 
	admin_id INTEGER, 
	sub_revoked_at DATETIME, 
	note VARCHAR(500), 
	online_at DATETIME, 
	on_hold_expire_duration BIGINT, 
	on_hold_timeout DATETIME, 
	auto_delete_in_days INTEGER, 
	hwid_limit BIGINT, 
	edit_at DATETIME, 
	last_status_change DATETIME, 
	created_at DATETIME NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_users PRIMARY KEY (id), 
	CONSTRAINT fk_users_admin_id_admins FOREIGN KEY(admin_id) REFERENCES admins (id)
);
INSERT INTO "users" VALUES('alice','{"vmess": {"id": "00000001-aaaa-4bbb-8ccc-000000000001"}, "vless": {"id": "00000001-aaaa-4bbb-8ccc-000000000001", "flow": "xtls-rprx-vision"}, "trojan": {"password": "trojan-pass-1"}, "shadowsocks": {"password": "ss-pass-1", "method": "chacha20-ietf-poly1305"}}','active',3221225472,10737418240,'month','2027-03-01 00:00:00.000000',2,NULL,'VIP customer','2026-10-22 18:05:00.000000',NULL,NULL,NULL,NULL,NULL,NULL,'2026-01-10 09:30:00.000000',1);
INSERT INTO "users" VALUES('bob_sh','{"vmess": {"id": "00000002-aaaa-4bbb-8ccc-000000000002"}, "vless": {"id": "00000002-aaaa-4bbb-8ccc-000000000002", "flow": ""}, "trojan": {"password": "trojan-pass-2"}, "shadowsocks": {"password": "ss-pass-2", "method": "chacha20-ietf-poly1305"}}','on_hold',0,NULL,'no_reset',NULL,NULL,NULL,NULL,NULL,2592000,'2026-12-01 00:00:00.000000',NULL,NULL,NULL,NULL,'2026-08-02 12:00:00.000000',2);
INSERT INTO "users" VALUES('carol','{"vmess": {"id": "00000003-aaaa-4bbb-8ccc-000000000003"}, "vless": {"id": "00000003-aaaa-4bbb-8ccc-000000000003", "flow": ""}, "trojan": {"password": "trojan-pass-3"}, "shadowsocks": {"password": "ss-pass-3", "method": "chacha20-ietf-poly1305"}}','limited',1288490188,1073741824,'no_reset',NULL,2,NULL,NULL,NULL,NULL,NULL,NULL,2,NULL,NULL,'2026-03-03 08:00:00.000000',3);
INSERT INTO "users" VALUES('dave','{"vmess": {"id": "00000004-aaaa-4bbb-8ccc-000000000004"}, "vless": {"id": "00000004-aaaa-4bbb-8ccc-000000000004", "flow": ""}, "trojan": {"password": "trojan-pass-4"}, "shadowsocks": {"password": "ss-pass-4", "method": "chacha20-ietf-poly1305"}}','expired',2147483648,5368709120,'no_reset','2026-09-01 00:00:00.000000',NULL,'2026-06-01 00:00:00.000000',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'2026-01-05 07:00:00.000000',4);
INSERT INTO "users" VALUES('eve.test','{"vmess": {"id": "00000037-aaaa-4bbb-8ccc-000000000037"}, "vless": {"id": "00000005-aaaa-4bbb-8ccc-000000000005", "flow": ""}, "trojan": {"password": "trojan-pass-5"}, "shadowsocks": {"password": "other-ss-pass", "method": "chacha20-ietf-poly1305"}}','disabled',0,NULL,'no_reset',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'2026-05-05 05:05:00.000000',5);
INSERT INTO "users" VALUES('frank','{"vmess": {"id": "00000006-aaaa-4bbb-8ccc-000000000006"}, "vless": {"id": "00000006-aaaa-4bbb-8ccc-000000000006", "flow": ""}, "trojan": {"password": "trojan-pass-6"}, "shadowsocks": {"password": "ss-pass-6", "method": "chacha20-ietf-poly1305"}}','active',1073741824,21474836480,'week',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'2026-02-14 14:00:00.000000',6);
CREATE TABLE users_groups_association (
	user_id INTEGER NOT NULL, 
	groups_id INTEGER NOT NULL, 
	CONSTRAINT pk_users_groups_association PRIMARY KEY (user_id, groups_id), 
	CONSTRAINT fk_users_groups_association_user_id_users FOREIGN KEY(user_id) REFERENCES users (id), 
	CONSTRAINT fk_users_groups_association_groups_id_groups FOREIGN KEY(groups_id) REFERENCES groups (id)
);
INSERT INTO "users_groups_association" VALUES(1,1);
INSERT INTO "users_groups_association" VALUES(3,1);
INSERT INTO "users_groups_association" VALUES(6,1);
INSERT INTO "users_groups_association" VALUES(1,2);
INSERT INTO "users_groups_association" VALUES(2,2);
INSERT INTO "users_groups_association" VALUES(4,2);
CREATE TABLE wireguard_subnets (
	network VARCHAR(64) NOT NULL, 
	next_offset INTEGER NOT NULL, 
	free_offsets JSON NOT NULL, 
	id INTEGER NOT NULL, 
	CONSTRAINT pk_wireguard_subnets PRIMARY KEY (id), 
	CONSTRAINT uq_wireguard_subnets_network UNIQUE (network)
);
CREATE UNIQUE INDEX ix_inbounds_tag ON inbounds (tag);
CREATE INDEX ix_client_templates_template_type ON client_templates (template_type);
CREATE UNIQUE INDEX ix_admins_username ON admins (username);
CREATE INDEX idx_users_admin_created ON users (admin_id, created_at);
CREATE UNIQUE INDEX ix_users_username ON users (username);
CREATE INDEX idx_users_admin_status ON users (admin_id, status);
CREATE INDEX idx_users_admin_online ON users (admin_id, online_at);
CREATE INDEX ix_node_usages_created_at ON node_usages (created_at);
CREATE INDEX ix_node_usage_reset_logs_node_id_created_at ON node_usage_reset_logs (node_id, created_at);
CREATE INDEX ix_admin_notification_reminders_admin_id_type ON admin_notification_reminders (admin_id, type);
CREATE INDEX ix_api_keys_expire_date ON api_keys (expire_date);
CREATE INDEX ix_api_keys_created_at ON api_keys (created_at);
CREATE INDEX ix_api_keys_admin_id ON api_keys (admin_id);
CREATE INDEX idx_user_subscription_updates_user_created ON user_subscription_updates (user_id, created_at);
CREATE INDEX idx_user_subscription_updates_user_id ON user_subscription_updates (user_id);
CREATE INDEX ix_user_hwids_user_id ON user_hwids (user_id);
CREATE INDEX ix_user_hwids_created_at ON user_hwids (created_at);
CREATE INDEX ix_user_hwids_hwid ON user_hwids (hwid);
CREATE INDEX ix_user_hwids_last_used_at ON user_hwids (last_used_at);
CREATE INDEX ix_next_plans_user_template_id ON next_plans (user_template_id);
CREATE INDEX ix_user_usage_logs_user_id_reset_at ON user_usage_logs (user_id, reset_at);
CREATE INDEX ix_node_user_usages_user_id_created_at ON node_user_usages (user_id, created_at);
CREATE INDEX ix_node_user_usages_created_at ON node_user_usages (created_at);
CREATE INDEX ix_node_user_usages_node_id_created_at ON node_user_usages (node_id, created_at);
COMMIT;
