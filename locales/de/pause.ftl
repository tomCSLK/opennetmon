# Pausieren und Fortsetzen von Gruppen, Geräten und Sensoren.
pause = Pausieren
resume = Fortsetzen
pause-heading = „{ $name }“ pausieren
pause-intro = Pausierte Sensoren werden nicht abgefragt und lösen keine Benachrichtigung aus. Beim Fortsetzen beginnen sie ohne alten Zustand neu.
pause-hint-group = Alle Untergruppen, Geräte und Sensoren der Gruppe werden mit pausiert.
pause-hint-device = Alle Sensoren des Geräts werden mit pausiert.
pause-duration = Dauer
pause-reason = Grund (optional)
pause-reason-hint = Erscheint in der Übersicht und im Protokoll, z. B. „Firmware-Update“.
pause-until-resumed = bis zum Fortsetzen
pause-minutes = { $count ->
    [one] 1 Minute
   *[other] { $count } Minuten
}
pause-hours = { $count ->
    [one] 1 Stunde
   *[other] { $count } Stunden
}
pause-days = { $count ->
    [one] 1 Tag
    [7] 1 Woche
   *[other] { $count } Tage
}
pause-until = bis
pause-note-own = Pausiert
pause-note-device = Pausiert mit dem Gerät
pause-note-group = Pausiert mit der Gruppe „{ $name }“
pause-device-notice = Solange das Gerät pausiert ist, wird keiner seiner Sensoren abgefragt.
pause-duration-invalid = Bitte eine der angebotenen Dauern wählen.
pause-reason-invalid = Der Grund darf höchstens 200 Zeichen lang sein und keine Zeilenumbrüche enthalten.
pause-not-paused = Das ist nicht (mehr) selbst pausiert.
