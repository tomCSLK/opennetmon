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
alert-subject-paused = Paused
alert-subject-many =
    { $count } status changes: { $parts }
alert-count-down = { $count } down
alert-dependents-paused = Paused by it: { $sensors ->
        [one] 1 sensor
       *[other] { $sensors } sensors
    } on { $devices ->
        [one] 1 device
       *[other] { $devices } devices
    }
alert-count-warning =
    { $count ->
        [one] { $count } warning
       *[other] { $count } warnings
    }
alert-count-up = { $count } OK again
mail-language-missing = Please choose the language of the e-mails.
mail-ui-url = Address of the web interface (optional)
mail-ui-url-hint = For buttons and links in the e-mails, e.g. https://opennetmon.example.org. Leave it empty and the e-mails contain no links.
mail-ui-url-invalid = The address of the web interface must start with http:// or https:// and must not contain credentials, a query (?) or a fragment (#).

# Content of the notification mails
mail-date = { $month }/{ $day }/{ $year }
mail-footer-sent =
    Sent by opennetmon { $version } to { $count ->
        [one] 1 recipient
       *[other] { $count } recipients
    }
mail-footer-zone = Times in { $zone }
mail-footer-batch = Changes within { $seconds } s come in one e-mail
mail-footer-settings = Notification settings
mail-open-sensor = Open sensor
mail-open-device = Open device
mail-open-log = Log
mail-open-report = 7-day report
mail-open-thresholds = Change limits
mail-open-overview = Open overview
mail-open-settings = Open settings
mail-since-time = since { $time }
mail-fact-since = Since
mail-fact-last-value = Last value
mail-fact-value = Current value
mail-fact-at = at { $time }
mail-fact-limits = Limits
mail-fact-availability = Availability, 7 days
mail-fact-outages = Outages, 7 days
mail-fact-down-lasted = Outage lasted
mail-fact-warning-lasted = Warning lasted
mail-fact-device = Device
mail-outages-none = no outage
mail-outages =
    { $count ->
        [one] 1 outage
       *[other] { $count } outages
    }, { $duration } in total
mail-outages-total = { $duration } in total
mail-limit-warn-above = Warning above { $value }
mail-limit-warn-below = Warning below { $value }
mail-limit-down-above = Down above { $value }
mail-limit-down-below = Down below { $value }
mail-chart-title = { $channel }, last 6 hours
mail-chart-max = Max. { $value }
mail-impact = Impact
mail-impact-paused = Paused by it (they depend on this sensor), no e-mails of their own:
mail-impact-sensors =
    { $count ->
        [one] 1 sensor
       *[other] { $count } sensors
    }
mail-impact-more =
    { $count ->
        [one] and 1 more device
       *[other] and { $count } more devices
    }
mail-impact-resumed =
    { $sensors ->
        [one] The paused sensor
       *[other] The { $sensors } paused sensors
    } on { $devices ->
        [one] 1 device
       *[other] { $devices } devices
    } ({ $names }) { $sensors ->
        [one] runs
       *[other] run
    } again; { $sensors ->
        [one] it is
       *[other] they are
    } polled as usual from now on.
mail-subject-down-lasted = down for { $duration }
mail-subject-warning-lasted = warning for { $duration }
mail-pre-up = OK again since { $time }.
mail-pre-down-lasted = The outage lasted { $duration }.
mail-pre-warning-lasted = The warning lasted { $duration }.
mail-batch-title =
    { $count ->
        [one] 1 status change
       *[other] { $count } status changes
    }
mail-batch-note = combined because they came shortly after each other
mail-counter-down =
    { $count ->
        [one] down
       *[other] down
    }
mail-counter-warning =
    { $count ->
        [one] warning
       *[other] warnings
    }
mail-counter-up = OK again
mail-cause = Probable cause
mail-cause-silent = They do not report on their own.
mail-all-changes = All changes
mail-more-changes = … and { $count } more
mail-test-title = The e-mail settings work
mail-test-ok = Test successful
mail-test-by = Sent by { $user } from Settings → E-mail
mail-test-pre = The e-mail settings work. This is what notifications look like.
mail-test-server = SMTP server
mail-test-login-yes = with login
mail-test-login-no = without login
mail-test-recipients = Recipients
mail-test-notify = Notified on
mail-test-notify-all = down, warning, recovery
mail-test-notify-down = down, recovery
mail-test-language = Language
mail-test-look = What notifications look like
mail-test-look-text = With key figures and the history of the last 6 hours, and with buttons into the web interface once its address is set. Changes that come shortly after each other are combined into one e-mail.
mail-duration-days =
    { $days ->
        [one] 1 day
       *[other] { $days } days
    }
mail-duration-days-hours =
    { $days ->
        [one] 1 day
       *[other] { $days } days
    } { $hours } h
mail-date-short = { $month }/{ $day }
mail-before-up = OK for { $duration } before
mail-before-warning = warning for { $duration } before
mail-before-down = down for { $duration } before
mail-before-unknown = unknown for { $duration } before
mail-before-paused = paused for { $duration } before
mail-fact-now = Now
mail-fact-average = 6-hour average: { $value }
mail-since-beyond = since { $time } { $limit }
mail-beyond-above = above { $value }
mail-beyond-below = below { $value }
mail-fact-trend = Trend
mail-trend = { $delta } in 2 h
mail-trend-before = before: { $range }
mail-fact-peak = Highest in 7 days
mail-fact-low = Lowest in 7 days
mail-peak-at = on { $date } at { $time }
mail-up-answers = Answers again – { $value }
mail-up-value = OK again – { $value }
mail-other-sensors = Other sensors of this device
mail-chart-warn-above = Warning { $value }
mail-chart-warn-below = Warning below { $value }
mail-chart-down-above = Down { $value }
mail-chart-down-below = Down below { $value }
mail-chart-no-answer = no answer
mail-chart-outage = down
mail-chart-alt = { $channel } over the last 6 hours
mail-chart-alt-down = { $channel } over the last 6 hours, { $state } since { $time }
mail-chart-alt-up = { $channel } over the last 6 hours with the outage from { $from } to { $to }
mail-chart-alt-warning = { $channel } over the last 6 hours, { $limit } since { $time }
