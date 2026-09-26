import React, { useState, useEffect } from 'react';

const Clock: React.FC = () => {
    const [time, setTime] = useState<Date>(new Date());

    useEffect(() => {
        // Function to update time
        const tick = () => setTime(new Date());

        // Set up an interval to update the clock every second
        const intervalId = setInterval(tick, 1000);

        // Clear the interval on component unmount
        return () => clearInterval(intervalId);
    }, []);

    // Format time as HH:MM:SS
    const formatTime = (date: Date) => {
        const hours = date.getHours().toString().padStart(2, '0');
        const minutes = date.getMinutes().toString().padStart(2, '0');
        const seconds = date.getSeconds().toString().padStart(2, '0');
        return `${hours}:${minutes}:${seconds}`;
    };

    return (
        <span style={styles.clock}>{formatTime(time)}</span>
    );
};

// Uses the design-system font token.
const styles = {
    clock: {
        fontFamily: 'var(--font-mono)',
        fontSize: '2rem',
        fontWeight: 700,
    },
};

export default Clock;