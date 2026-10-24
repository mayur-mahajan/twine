# Arabic (Fluent): six plural categories, right to left.
hello = مرحبا
greeting = مرحبا يا { $name }!
items = { $count ->
    [zero] لا عناصر
    [one] عنصر واحد
    [two] عنصران
    [few] { $count } عناصر
    [many] { $count } عنصرًا
   *[other] { $count } عنصر
}
