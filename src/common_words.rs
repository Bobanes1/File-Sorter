//! Everyday words that never make good keywords for one module: frequent German and
//! English words, and words that show up in every subject's lecture material.
//!
//! Without such a list, suggestions are drowned in words like "heute", "Beispiel" or
//! "Ergebnis", which are frequent in a module's files but say nothing about the module.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::matcher::normalise;

const GERMAN: &str = "
aber ab abend achtung acht ähnlich ähnliche ähnlichen alle allein allem allen aller alles allgemein allgemeine
allgemeinen also alt alte alten als am an andere anderen anderer anderes anders anfang angabe angaben
angegeben ansatz ansätze ansicht anteil antwort antworten anwendung anwendungen anzahl arbeit arbeiten
art arten artikel aspekt aspekte auch auf aufbau aufgabe aufgaben aufgabenstellung aus ausgabe ausgewählte
auswahl aussage aussagen außen außer ausserdem außerdem bald basis bedeutung bedeutet bei beide beiden
beim beispiel beispiele beispielen beispielsweise bekannt bekannte bemerkung bemerkungen bereich bereiche
bereits berechnen berechnung bericht besonders besser beste bestimmen bestimmt bestimmte bestimmten betrachten
betrachtet betrachtung beträgt bevor bewertung bezug bild bilder bin bis bisher bitte bleibt brauchen
bzw ca dabei dadurch dafür daher damit danach dann daran darauf darstellung darum das dass daten
datei dateien dazu definieren definiert definition definitionen dem demnach den denen denn dennoch der
deren derzeit des deshalb dessen detail details deutlich dich die dienstag dies diese diesem diesen dieser
dieses dir direkt doch donnerstag dort drei dritte du durch dürfen eben ebenfalls ebenso egal eher eigene
eigenen eigentlich ein eine einem einen einer eines einfach einfache einfachen einführung eingabe einige
einigen einmal eins einzelne einzelnen ende endlich entsprechend entsprechende entweder er ergebnis
ergebnisse ergibt erhalten erkennen erklären erklärung erst erste ersten erster erstes es etwa etwas
euch euer fach fakultät fall fälle falls falsch fehler finden folgen folgende folgenden folgt form formen
frage fragen freitag früh für fünf ganz ganze ganzen gar geben gebiet gegeben gegen gegenüber gehen
gehört gemacht gemeinsam genau genannt genug gerade gering gesamt geschrieben gesehen gestellt gibt gilt
gleich gleichen grad grafik grosse große großen grossen grund grundlage grundlagen gruppe gruppen gut gute
guten habe haben halb hälfte hand hat hatte hätte hauptsächlich heisst heißt heute hier hierbei hilfe
hin hinweis hinweise hoch hohe hohen ich ideen ihm ihn ihnen ihr ihre ihrem ihren ihrer im immer in
indem information informationen inhalt inhalte innerhalb insbesondere insgesamt interessant ist ja jahr
jahre jahren je jede jedem jeden jeder jedes jedoch jetzt kann kapitel kein keine keinen klar klein kleine
kleinen kommen kommt können konnte könnte kurs kurz kurze lage lang lange lassen laut leicht lernen
lernziele letzte letzten leute liegt links liste machen macht mal man manche manchmal mehr mehrere mein
meine meist meisten menge methode methoden mich mindestens mir mit mithilfe mittel mittwoch möglich mögliche
möglichen möglichkeit möglichkeiten modell modelle moment montag morgen muss müssen musste nach nachdem
nächste nächsten nahe name namen neben nehmen nein neu neue neuen nicht nichts nie noch nochmals notwendig
nr nummer nun nur ob oben oder oft ohne ort paar person personen praktisch problem probleme prozent
prozess prozesse punkt punkte rahmen recht rechts regel regeln relevant richtig rolle sache sagen samstag
satz schnell schon schritt schritte schwer schwierig sechs sehen sehr sei seid sein seine seinem seinen
seiner seit seite seiten selbst semester sich sicher sie sieben sieht sind sinn situation so sodass sofort
sogar sollen sollte sollten sondern sonntag sonst sowie sowohl spät später sprechen stand stark statt
stehen steht stellen stelle stellt struktur strukturen stück stunde stunden suche system systeme tag tage
tagen teil teile teilen thema themen theorie tun typ typen über überblick überhaupt übersicht übung übungen
übungsblatt uebung uebungen uebungsblatt uhr um umfang und uns unser unsere unter unterschied unterschiede
unterschiedlich unterschiedliche usw verfahren vergleich verhältnis verschiedene verschiedenen version
verstehen verwendet verwendung viel viele vielen vielleicht vier vom von vor vorgehen vorher vorlesung
vorlesungen vorstellung während wahr wann war waren warum was weg weil weise weiter weitere weiteren welche
welchen welcher welches wenig wenige wenn wer werden wert werte wichtig wichtige wichtigen wie wieder
wieso will wir wird wirklich wissen wo woche wochen wohl wollen wort worte wörter wurde wurden ziel ziele
zeigen zeigt zeit zeiten zu zum zur zurück zusammen zusammenfassung zusätzlich zwar zwei zweite zweiten
zwischen
anmerkung anmerkungen anhang abbildung abbildungen abgabe abgabetermin beachten bearbeiten bearbeitung
beschreibung beschreiben blatt blätter campus dozent dozentin dozierende einleitung erläuterung fazit
folie folien frist gliederung hausaufgabe hausaufgaben hörsaal inhaltsverzeichnis institut klausur klausuren
kommentar kommentare leistungsnachweis lehrstuhl literatur lösung lösungen lösungsvorschlag modul module
musterlösung note noten online organisatorisches präsentation professor professorin prüfung prüfungen
quelle quellen referat seminar skript sitzung sprechstunde student studenten studentin studierende studium
tabelle tabellen termin termine tutorat tutorium universität uni veranstaltung wintersemester sommersemester
hs fs ws ss
";

const ENGLISH: &str = "
a about above according actually after again against all almost along already also although always am
among an and another any anything are area around as ask asked at available away back based be because
become been before being below best better between both but by call called can case cases certain change
chapter class come common consider could course data day days define defined definition did different
do does done down during each early easy either else end enough even every example examples fact far few
figure find first five following for form found four from full further general get give given go good great
group had has have he her here high him his how however i idea if important in include including information
instead into introduction is issue it its just keep kind know known large last later least left less let
level like line list little long look lot made main make many may me mean means method methods might more
most much must my name need new next no not note notes now number of off often old on once one only open
or order other our out over overview own page pages part people per place point points possible present
problem problems process provide put question questions quite rather read real really result results right
same say second section see seen set several shall she should show shown side simple since small so some
something state step steps still such summary sure system table take task tasks than that the their them
then there these they thing things think third this those three through thus time times to today together
too topic topics total true try two type types under understand until up upon us use used useful uses using
usually value values very via want was way ways we week weeks well were what when where whether which while
who whole why will with within without word words work would write year years yes yet you your
assignment assignments exam exams exercise exercises homework lecture lectures lecturer professor quiz
reading readings seminar session sessions sheet slide slides solution solutions student students syllabus
tutorial university semester term deadline submission grade grades
";

/// The normalised everyday words.
pub fn common_words() -> &'static HashSet<String> {
    static WORDS: OnceLock<HashSet<String>> = OnceLock::new();
    WORDS.get_or_init(|| {
        GERMAN.split_whitespace().chain(ENGLISH.split_whitespace()).map(normalise).filter(|w| !w.is_empty()).collect()
    })
}

pub fn is_common(word: &str) -> bool {
    common_words().contains(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyday_words_are_known_in_normalised_form() {
        for word in ["heute", "beispiel", "ergebnis", "uber", "ubungsblatt", "uebungsblatt", "grosse", "the", "result"]
        {
            assert!(is_common(word), "{word}");
        }
        for word in ["rekursion", "varianz", "regression", "huber", "matrix", "agenda"] {
            assert!(!is_common(word), "{word}");
        }
    }
}
