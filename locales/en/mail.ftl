mail-security-none = No encryption
mail-security-starttls = STARTTLS
mail-security-tls = TLS (SMTPS)
mail-too-many-recipients = At most { $count } recipients are allowed.
mail-address-invalid = “{ $address }” is not a valid e-mail address.
mail-port-invalid = The port must be a number between 1 and 65535.
mail-username-invalid = The user name is invalid.
mail-password-invalid = The password is invalid (too long or control characters).
mail-host-missing = Please enter the SMTP server.
mail-from-missing = Please enter a sender address.
mail-recipients-missing = Please enter at least one recipient.
mail-send-failed = Delivery failed: { $detail }
mail-from-invalid = The sender address is not valid.
mail-no-recipients = No recipient is set.
mail-recipient-invalid = A recipient address is not valid.
mail-message-invalid = The message is invalid: { $detail }
mail-server-unusable = The SMTP server cannot be used: { $detail }
mail-no-server = No SMTP server is set.
mail-timeout = Timeout while sending.
mail-kind-alert = Alert
mail-kind-test = Test
mail-security-missing = Please choose the encryption.
mail-test-subject = Test e-mail
mail-test-body =
    This is a test e-mail from opennetmon.
    If it arrives, server, credentials and recipients are set up correctly.
mail-test-sent = Test e-mail sent to { $recipients }.
mail-title = E-mail notification
mail-subtitle = An e-mail is sent on failure, warning and recovery. Changes that happen shortly after each other are combined. Changes apply at once.
mail-enabled = Notifications on
mail-notify-warnings = Also notify on warnings (otherwise only on failure and recovery)
mail-smtp-server = SMTP server
mail-host = Server (host name or IP address)
mail-security = Encryption
mail-port-hint = Usual: STARTTLS 587, TLS (SMTPS) 465, unencrypted 25 (only for a trusted local relay).
mail-login = Login (optional)
mail-username-placeholder = empty = no login
mail-not-set = not set
mail-from-to = Sender and recipients
mail-from = Sender address
mail-recipients = Recipients (one address per line, at most 20)
mail-send-test = Send test e-mail
mail-test-hint = Uses the saved settings (save first, then test) – even while notifications are still off.
mail-log = Recent deliveries
mail-log-empty = No e-mails sent yet.
mail-log-kind = Kind
mail-log-subject = Subject
mail-log-recipients = Recipients
mail-log-result = Result
mail-sent = sent
mail-failed = failed

# Notification mails (in the language chosen on the e-mail page)
mail-language = Language of the e-mails
mail-language-hint = Applies to all recipients and to the test e-mail.
alert-subject-down = Down
alert-subject-warning = Warning
alert-subject-up = OK again
alert-subject-unknown = Unknown
alert-subject-paused = Inactive
alert-subject-many =
    { $count } status changes: { $parts }
alert-count-down = { $count } down
alert-count-warning =
    { $count ->
        [one] { $count } warning
       *[other] { $count } warnings
    }
alert-count-up = { $count } OK again
alert-first-poll = first poll
mail-language-missing = Please choose the language of the e-mails.
