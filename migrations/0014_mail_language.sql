-- Language of the notification mails (`de`, `en`): one language for all recipients.
ALTER TABLE mail_settings ADD COLUMN language TEXT NOT NULL DEFAULT 'de';
