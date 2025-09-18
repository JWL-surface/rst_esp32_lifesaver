# rst_esp32_lifesaver
an embedded rust repo for controlling the ESP32 to read data from the analog front end and send it up via mqtt over wifi. 

## SAS Token
To generate the SAS token correctly, the machine running `server.py` must set environment variable `IOTHUB_DEVICE_KEY = "device key"`.
