-- Address of the web interface for buttons and links in notification mails (optional; stored
-- without a trailing slash). Without it, mails carry no links.
ALTER TABLE mail_settings ADD COLUMN ui_url TEXT;
