I want to implement logger like nginx in this application.

I want you to create an axum server which has one endpoint:
Endpoint:
/       : This should return a webpage which has a form.
/add    : This accepts name and age and puts entry in a in memory map

I should be able to render the webpage in brave browser if I access root of webpage.
Form should have two fields name and age and a save button. 
On click save button age and name should be sent to /add endpoint and stored in memory hash map.

Here is the logging part I want to implement:

1. Log all lines in debug mode and success in info.
2. I should be able to change log level using a signal to the application. Use SIGHUP1 update from debug to info back and forth.


Think like a Mozilla engineer who is well versed in Rust.



