import matplotlib.pyplot as plt
import numpy as np


def moving_average(a, n=10):
    ret = np.cumsum(a, dtype=float)
    ret[n:] = ret[n:] - ret[:-n]
    return ret[n - 1:] / n


# Replace this with the path to your file
filename = 'plot4.txt'

# Read the file and convert each line to a float
with open(filename, 'r') as file:
    data = [float(line.strip()) for line in file if line.strip()]

data = np.array(data)
avg = moving_average(data, n=1)
# Create a simple line plot
plt.plot(avg)
plt.xlabel('Index')
plt.ylabel('mV')
plt.title('heart rate')
plt.grid(True)
plt.show()
